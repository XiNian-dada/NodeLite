"""Encrypted per-custodian Ed25519 keys and detached approvals bound to exact bytes."""

import getpass
import os
from pathlib import Path
import stat

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from securesystemslib.signer import CryptoSigner, Signature
from tuf.api.metadata import Metadata

from .io import create, decode, digest, json_bytes, owned_directory, read


def password(fd=None):
    if fd is None:
        value = getpass.getpass("Signing key passphrase: ").encode()
    else:
        with os.fdopen(os.dup(fd), "rb") as stream:
            value = stream.readline(1026).rstrip(b"\r\n")
    if not 12 <= len(value) <= 1024:
        raise ValueError("passphrase must contain 12..1024 bytes")
    return value


def generate(private, public, secret):
    private, public = Path(private), Path(public)
    owned_directory(private.parent, private=True)
    if private.exists() or private.is_symlink() or public.exists() or public.is_symlink():
        raise ValueError("key output already exists")
    key = Ed25519PrivateKey.generate()
    signer = CryptoSigner(key)
    encrypted = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                 serialization.BestAvailableEncryption(secret))
    create(private, encrypted, 0o600)
    create(public, json_bytes({"keyid": signer.public_key.keyid, "key": signer.public_key.to_dict()}))
    return {"keyid": signer.public_key.keyid, "encrypted": True}


def approve(metadata, private, output, secret):
    path = Path(private)
    owned_directory(path.parent, private=True)
    info = path.lstat()
    if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise ValueError("private key must be owner-only mode 0600")
    key = serialization.load_pem_private_key(read(path, 16384), password=secret)
    if not isinstance(key, Ed25519PrivateKey):
        raise ValueError("only encrypted Ed25519 keys are supported")
    raw = read(metadata)
    decode(raw)
    document = Metadata.from_bytes(raw)
    signature = document.sign(CryptoSigner(key))
    create(output, json_bytes({"schema": 1, "role": document.signed.type,
        "payload_sha256": digest(document.signed_bytes), "signature": signature.to_dict()}))
    return {"role": document.signed.type, "keyid": signature.keyid,
            "payload_sha256": digest(document.signed_bytes)}


def combine(document, approvals):
    document.signatures.clear()
    for path in approvals:
        approval = decode(read(path, 16384))
        if set(approval) != {"schema", "role", "payload_sha256", "signature"} \
                or approval["schema"] != 1 or approval["role"] != document.signed.type \
                or approval["payload_sha256"] != digest(document.signed_bytes):
            raise ValueError("approval is not for this exact role payload")
        signature = Signature.from_dict(approval["signature"])
        if signature.keyid in document.signatures:
            raise ValueError("duplicate approval cannot count toward threshold")
        document.signatures[signature.keyid] = signature
    return document
