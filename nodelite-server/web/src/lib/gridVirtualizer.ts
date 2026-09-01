export const NODE_CARD_HEIGHT = 282;
export const NODE_GRID_GAP = 14;
export const NODE_GRID_MIN_COLUMN_WIDTH = 320;
export const NODE_GRID_OVERSCAN_ROWS = 2;

export interface GridVirtualWindow {
  columns: number;
  totalRows: number;
  startRow: number;
  endRow: number;
  startIndex: number;
  endIndex: number;
  offsetTop: number;
  totalHeight: number;
}

export function gridColumnCount(containerWidth: number, viewportWidth: number): number {
  if (viewportWidth >= 1920) return 4;
  const width = Number.isFinite(containerWidth) && containerWidth > 0 ? containerWidth : 1;
  return Math.max(
    1,
    Math.floor((width + NODE_GRID_GAP) / (NODE_GRID_MIN_COLUMN_WIDTH + NODE_GRID_GAP)),
  );
}

export function gridVirtualWindow(
  itemCount: number,
  columns: number,
  scrollTop: number,
  viewportHeight: number,
): GridVirtualWindow {
  const safeCount = Math.max(0, Math.floor(itemCount));
  const safeColumns = Math.max(1, Math.floor(columns));
  const totalRows = Math.ceil(safeCount / safeColumns);
  const rowStride = NODE_CARD_HEIGHT + NODE_GRID_GAP;
  const startRow = Math.min(
    totalRows,
    Math.max(0, Math.floor(scrollTop / rowStride) - NODE_GRID_OVERSCAN_ROWS),
  );
  const endRow = Math.min(
    totalRows,
    Math.max(
      startRow,
      Math.ceil((scrollTop + Math.max(0, viewportHeight)) / rowStride) + NODE_GRID_OVERSCAN_ROWS,
    ),
  );

  return {
    columns: safeColumns,
    totalRows,
    startRow,
    endRow,
    startIndex: Math.min(safeCount, startRow * safeColumns),
    endIndex: Math.min(safeCount, endRow * safeColumns),
    offsetTop: startRow * rowStride,
    totalHeight: totalRows === 0 ? 0 : totalRows * rowStride - NODE_GRID_GAP,
  };
}
