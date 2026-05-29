/**
 * Pure tag parsers for node identity metadata, ported from
 * assets/node.html:1694-1709. Nodes encode IP/location as `key:value` tags.
 */

import type { NodeStatus } from '@/api';

/** Location from a `loc|location|region|city:value` tag, else null. */
export function locationFromNode(node: NodeStatus): string | null {
  for (const tag of node.identity.tags || []) {
    const m = String(tag).match(/^(?:loc|location|region|city)[:=](.+)$/i);
    if (m && m[1]) return m[1];
  }
  return null;
}

/** IP from an `ip|addr|address:value` tag, falling back to remote_ip. */
export function ipFromNode(node: NodeStatus): string | null {
  for (const tag of node.identity.tags || []) {
    const m = String(tag).match(/^(?:ip|addr|address)[:=](.+)$/i);
    if (m && m[1]) return m[1];
  }
  return node.remote_ip || null;
}
