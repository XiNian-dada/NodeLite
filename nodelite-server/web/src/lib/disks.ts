/**
 * Disk helpers ported from node.html:1838. The agent can report the same
 * device twice; dedupe by device + total size. Pure.
 */

import type { DiskUsage } from '@/api';

export function uniqueDisks(disks: DiskUsage[] | undefined): DiskUsage[] {
  const seen = new Set<string>();
  return (Array.isArray(disks) ? disks : []).filter((disk) => {
    const key = `${disk.device || ''}:${disk.total_bytes || 0}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function totalDiskBytes(disks: DiskUsage[]): number {
  return disks.reduce((sum, d) => sum + (d.total_bytes || 0), 0);
}

export function usedDiskBytes(disks: DiskUsage[]): number {
  return disks.reduce((sum, d) => sum + (d.used_bytes || 0), 0);
}
