/**
 * Typed wrappers per legacy endpoint inventory. Response shapes live in
 * ./types so components can import them without dragging in the client.
 */

import { api } from './client';
import type {
  AgentLogEntry,
  BootstrapResponse,
  ChangePasswordRequest,
  DisableTwoFactorRequest,
  EnableTwoFactorRequest,
  HistoryPoint,
  HistoryQuery,
  NodeListItem,
  NodeStatus,
  OverviewData,
  ReauthPayload,
  SettingsActionResponse,
  SettingsResponse,
  TwoFactorSetupResponse,
} from './types';

export type {
  AgentLogEntry,
  BootstrapResponse,
  ChangePasswordRequest,
  DisableTwoFactorRequest,
  DiskUsage,
  EnableTwoFactorRequest,
  HistoryPoint,
  HistoryQuery,
  LogLevel,
  NodeIdentity,
  NodeListItem,
  NodeListIdentity,
  NodeListSnapshot,
  NodeSnapshot,
  NodeStatus,
  OverviewData,
  ReauthPayload,
  SettingsActionResponse,
  SettingsAgentToken,
  SettingsAuth,
  SettingsResponse,
  SettingsUpdates,
  TwoFactorSetupResponse,
} from './types';

function postJson<T>(path: string, body: unknown): Promise<T> {
  return api<T>(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export const apiClient = {
  bootstrap: () => api<BootstrapResponse>('/api/bootstrap'),
  overview: () => api<OverviewData>('/api/overview'),
  listNodes: () => api<NodeListItem[]>('/api/nodes'),
  /** Full per-node status (NodeStatus), not the lightweight list shape. */
  nodeStatus: (id: string) => api<NodeStatus>(`/api/nodes/${encodeURIComponent(id)}`),
  nodeHistory: (id: string, query: HistoryQuery = {}) => {
    const params = new URLSearchParams();
    if (query.windowHours !== undefined) {
      params.set('window_hours', String(query.windowHours));
    }
    if (query.maxPoints !== undefined) {
      params.set('max_points', String(query.maxPoints));
    }
    if (query.start !== undefined) {
      params.set('start', String(query.start));
    }
    if (query.end !== undefined) {
      params.set('end', String(query.end));
    }
    const qs = params.toString();
    const suffix = qs ? `?${qs}` : '';
    return api<HistoryPoint[]>(`/api/nodes/${encodeURIComponent(id)}/history${suffix}`);
  },
  nodeLogs: (id: string, limit = 200) =>
    api<AgentLogEntry[]>(
      `/api/nodes/${encodeURIComponent(id)}/logs?limit=${encodeURIComponent(String(limit))}`,
    ),

  // --- Settings ---
  settings: () => api<SettingsResponse>('/api/settings'),
  updateServer: (body: ReauthPayload) =>
    postJson<SettingsActionResponse>('/api/settings/update/server', body),

  // --- Account / security ---
  twoFactorStart: () =>
    postJson<TwoFactorSetupResponse>('/api/settings/2fa/start', {}),
  twoFactorEnable: (body: EnableTwoFactorRequest) =>
    postJson<SettingsActionResponse>('/api/settings/2fa/enable', body),
  twoFactorDisable: (body: DisableTwoFactorRequest) =>
    postJson<SettingsActionResponse>('/api/settings/2fa/disable', body),
  changePassword: (body: ChangePasswordRequest) =>
    postJson<SettingsActionResponse>('/api/settings/password', body),
};
