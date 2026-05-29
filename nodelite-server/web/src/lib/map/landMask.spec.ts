import { describe, expect, it, vi } from 'vitest';
import {
  WORLD_LAND_POLYGONS,
  drawFallbackMask,
  drawGeoJsonMask,
  drawProjectedRing,
  polygonMaxLat,
  sampleDots,
  type GeoJsonFeatureCollection,
} from './landMask';

function mockCtx() {
  return {
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    fill: vi.fn(),
  } as unknown as CanvasRenderingContext2D & {
    beginPath: ReturnType<typeof vi.fn>;
    moveTo: ReturnType<typeof vi.fn>;
    lineTo: ReturnType<typeof vi.fn>;
    fill: ReturnType<typeof vi.fn>;
  };
}

describe('polygonMaxLat', () => {
  it('returns the maximum latitude across all rings', () => {
    expect(polygonMaxLat([[[0, 10], [5, 40], [10, -3]]])).toBe(40);
  });

  it('returns -90 for an empty polygon', () => {
    expect(polygonMaxLat([])).toBe(-90);
  });
});

describe('drawProjectedRing', () => {
  it('moves to the first point then lines to the rest', () => {
    const ctx = mockCtx();
    drawProjectedRing(ctx, [[0, 0], [10, 0], [20, 10]]);
    expect(ctx.moveTo).toHaveBeenCalledTimes(1);
    expect(ctx.lineTo).toHaveBeenCalledTimes(2);
  });

  it('starts a new subpath across the antimeridian (>180° jump)', () => {
    const ctx = mockCtx();
    drawProjectedRing(ctx, [[170, 0], [-170, 0], [-160, 0]]);
    // jump 170 -> -170 is 340° > 180 so it moveTo's again
    expect(ctx.moveTo).toHaveBeenCalledTimes(2);
    expect(ctx.lineTo).toHaveBeenCalledTimes(1);
  });

  it('skips non-finite coordinates', () => {
    const ctx = mockCtx();
    drawProjectedRing(ctx, [[0, 0], [Number.NaN, 5], [10, 10]]);
    expect(ctx.moveTo).toHaveBeenCalledTimes(1);
    expect(ctx.lineTo).toHaveBeenCalledTimes(1);
  });
});

describe('drawGeoJsonMask', () => {
  it('draws Polygon + MultiPolygon features and fills evenodd', () => {
    const ctx = mockCtx();
    const geo: GeoJsonFeatureCollection = {
      features: [
        { geometry: { type: 'Polygon', coordinates: [[[0, 10], [5, 12], [3, 8]]] } },
        {
          geometry: {
            type: 'MultiPolygon',
            coordinates: [[[[20, 10], [25, 12], [23, 8]]]],
          },
        },
      ],
    };
    drawGeoJsonMask(ctx, geo);
    expect(ctx.beginPath).toHaveBeenCalledOnce();
    expect(ctx.moveTo).toHaveBeenCalledTimes(2);
    expect(ctx.fill).toHaveBeenCalledWith('evenodd');
  });

  it('skips polygons entirely below the min latitude', () => {
    const ctx = mockCtx();
    const geo: GeoJsonFeatureCollection = {
      features: [
        { geometry: { type: 'Polygon', coordinates: [[[0, -80], [5, -82], [3, -85]]] } },
      ],
    };
    drawGeoJsonMask(ctx, geo);
    expect(ctx.moveTo).not.toHaveBeenCalled();
  });
});

describe('drawFallbackMask', () => {
  it('draws every built-in polygon and fills', () => {
    const ctx = mockCtx();
    drawFallbackMask(ctx);
    expect(ctx.beginPath).toHaveBeenCalledOnce();
    // one moveTo per polygon (first vertex of each ring)
    expect(ctx.moveTo).toHaveBeenCalledTimes(WORLD_LAND_POLYGONS.length);
    expect(ctx.fill).toHaveBeenCalledWith('evenodd');
  });
});

describe('sampleDots', () => {
  it('emits a dot for land pixels and skips transparent-black ones', () => {
    // 4x4 grid, gap 2 → sampled at (0,0),(2,0),(0,2),(2,2)
    const width = 4;
    const height = 4;
    const pixels = new Uint8ClampedArray(width * height * 4); // all transparent black
    // mark pixel (x=2, y=0) as land (white opaque)
    const landIndex = (0 * width + 2) * 4;
    pixels[landIndex] = 255;
    pixels[landIndex + 3] = 255;

    const dots = sampleDots(pixels, width, height, 2);
    expect(dots).toContainEqual({ x: 2, y: 0 });
    // the all-zero samples are skipped
    expect(dots).not.toContainEqual({ x: 0, y: 0 });
  });

  it('keeps a pixel that is opaque even if red channel is 0', () => {
    const width = 2;
    const height = 1;
    const pixels = new Uint8ClampedArray(width * height * 4);
    // (0,0): r=0 but alpha=255 → not (0 && 0) → kept
    pixels[3] = 255;
    const dots = sampleDots(pixels, width, height, 2);
    expect(dots).toContainEqual({ x: 0, y: 0 });
  });
});
