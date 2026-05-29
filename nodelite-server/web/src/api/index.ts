/**
 * Typed wrappers per legacy endpoint inventory. Response shapes live in
 * ./types so components can import them without dragging in the client.
 */

import { api } from './client';
import type {
  BootstrapResponse,
  HistoryPoint,
  HistoryQuery,
  NodeListItem,
  OverviewData,
} from './types';

export type {
  BootstrapResponse,
  HistoryPoint,
  HistoryQuery,
  NodeListItem,
  NodeListIdentity,
  NodeListSnapshot,
  OverviewData,
} from './types';

export const apiClient = {
  bootstrap: () => api<BootstrapResponse>('/api/bootstrap'),
  overview: () => api<OverviewData>('/api/overview'),
  listNodes: () => api<NodeListItem[]>('/api/nodes'),
  getNode: (id: string) => api<NodeListItem>(`/api/nodes/${encodeURIComponent(id)}`),
  nodeHistory: (id: string, query: HistoryQuery = {}) => {
    const params = new URLSearchParams();
    if (query.windowHours !== undefined) {
      params.set('window_hours', String(query.windowHours));
    }
    if (query.maxPoints !== undefined) {
      params.set('max_points', String(query.maxPoints));
    }
    const qs = params.toString();
    const suffix = qs ? `?${qs}` : '';
    return api<HistoryPoint[]>(
      `/api/nodes/${encodeURIComponent(id)}/history${suffix}`,
    );
  },
};
