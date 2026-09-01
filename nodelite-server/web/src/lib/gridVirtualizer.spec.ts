import { describe, expect, it } from 'vitest';
import {
  gridColumnCount,
  gridVirtualWindow,
  NODE_CARD_HEIGHT,
  NODE_GRID_GAP,
} from './gridVirtualizer';

describe('gridColumnCount', () => {
  it('matches the responsive card grid', () => {
    expect(gridColumnCount(319, 1280)).toBe(1);
    expect(gridColumnCount(654, 1280)).toBe(2);
    expect(gridColumnCount(1200, 1280)).toBe(3);
    expect(gridColumnCount(1800, 1800)).toBe(5);
    expect(gridColumnCount(1800, 1920)).toBe(4);
  });
});

describe('gridVirtualWindow', () => {
  it('limits a 500-node grid to viewport rows plus overscan', () => {
    const window = gridVirtualWindow(500, 3, 0, 800);
    expect(window.startIndex).toBe(0);
    expect(window.endIndex).toBe(15);
    expect(window.totalRows).toBe(167);
    expect(window.totalHeight).toBe(167 * (NODE_CARD_HEIGHT + NODE_GRID_GAP) - NODE_GRID_GAP);
  });

  it('moves the item window by whole grid rows', () => {
    const rowStride = NODE_CARD_HEIGHT + NODE_GRID_GAP;
    const window = gridVirtualWindow(500, 3, rowStride * 20, 800);
    expect(window.startRow).toBe(18);
    expect(window.startIndex).toBe(54);
    expect(window.endIndex - window.startIndex).toBeLessThan(50);
    expect(window.offsetTop).toBe(rowStride * 18);
  });

  it('clamps empty and past-the-end windows', () => {
    expect(gridVirtualWindow(0, 0, 0, 800)).toMatchObject({
      totalRows: 0,
      startIndex: 0,
      endIndex: 0,
      totalHeight: 0,
    });
    expect(gridVirtualWindow(5, 2, 99_999, 800)).toMatchObject({
      startIndex: 5,
      endIndex: 5,
    });
  });
});
