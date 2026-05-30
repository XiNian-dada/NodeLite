import { describe, expect, it } from 'vitest';
import type { DiskUsage } from '@/api';
import { totalDiskBytes, uniqueDisks, usedDiskBytes } from './disks';

function disk(over: Partial<DiskUsage>): DiskUsage {
  return {
    device: '/dev/sda1',
    mount_point: '/',
    fs_type: 'ext4',
    total_bytes: 100,
    available_bytes: 60,
    used_bytes: 40,
    used_percent: 40,
    ...over,
  };
}

describe('uniqueDisks', () => {
  it('dedupes by device + total size', () => {
    const out = uniqueDisks([
      disk({ device: '/dev/sda1', total_bytes: 100 }),
      disk({ device: '/dev/sda1', total_bytes: 100 }), // dup
      disk({ device: '/dev/sdb1', total_bytes: 200 }),
    ]);
    expect(out).toHaveLength(2);
  });

  it('returns [] for undefined', () => {
    expect(uniqueDisks(undefined)).toEqual([]);
  });
});

describe('totalDiskBytes / usedDiskBytes', () => {
  it('sums across disks', () => {
    const disks = [disk({ total_bytes: 100, used_bytes: 40 }), disk({ total_bytes: 200, used_bytes: 50 })];
    expect(totalDiskBytes(disks)).toBe(300);
    expect(usedDiskBytes(disks)).toBe(90);
  });
});
