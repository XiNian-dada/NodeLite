/**
 * Typed wrappers per legacy endpoint inventory.
 *
 * Response types are hand-written best-effort — full type generation from
 * nodelite-proto is deferred (plan §6.2). When Stage 2/3 components hit a
 * field that's not yet typed, widen here rather than reaching for `any`.
 */

import { api } from './client';

export interface BootstrapResponse {
  /** Polling interval in milliseconds */
  refreshIntervalMs?: number;
  /** Server display version */
  version?: string;
  /** Any other server-provided bootstrap fields (typed in Stage 2) */
  [key: string]: unknown;
}

export interface OverviewResponse {
  [key: string]: unknown;
}

export interface NodeSummary {
  id: string;
  name?: string;
  [key: string]: unknown;
}

export const apiClient = {
  bootstrap: () => api<BootstrapResponse>('/api/bootstrap'),
  overview: () => api<OverviewResponse>('/api/overview'),
  listNodes: () => api<NodeSummary[]>('/api/nodes'),
  getNode: (id: string) =>
    api<NodeSummary>(`/api/nodes/${encodeURIComponent(id)}`),
};
