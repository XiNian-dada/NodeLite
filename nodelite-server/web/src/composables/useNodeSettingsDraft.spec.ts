import { describe, expect, it } from 'vitest';
import { makeSettings } from '@/api/__fixtures__/nodes';
import {
  dateInputValue,
  optionalNumber,
  reauthBody,
  serviceExpiresAt,
  syncDraftsFromAgent,
  type LocationDraft,
  type ServiceDraft,
} from './useNodeSettingsDraft';

describe('useNodeSettingsDraft helpers', () => {
  it('normalizes service dates for date inputs and API payloads', () => {
    expect(dateInputValue(null)).toBe('');
    expect(dateInputValue('2026-12-31T00:00:00Z')).toBe('2026-12-31');
    expect(dateInputValue('2026-12-31 manually entered')).toBe('2026-12-31');
    expect(dateInputValue('not a date')).toBe('');
    expect(serviceExpiresAt('')).toBeNull();
    expect(serviceExpiresAt('2027-01-15')).toBe('2027-01-15T00:00:00Z');
  });

  it('parses optional numeric location fields', () => {
    expect(optionalNumber('')).toBeNull();
    expect(optionalNumber(' 22.3193 ')).toBe(22.3193);
    expect(optionalNumber('abc')).toBeUndefined();
  });

  it('omits blank reauth fields from refresh payloads', () => {
    expect(reauthBody({ current_password: '', code: '' })).toEqual({});
    expect(reauthBody({ current_password: 'hunter2', code: '' })).toEqual({
      current_password: 'hunter2',
    });
    expect(reauthBody({ current_password: '', code: '123456' })).toEqual({ code: '123456' });
  });

  it('syncs service and location drafts when the selected agent changes', () => {
    const agent = makeSettings({
      agents: [
        {
          ...makeSettings().agents[0]!,
          service_expires_at: '2026-12-31T00:00:00Z',
          service_unlimited: true,
          renewal_price: '$9/mo',
          location_override_country: 'HK',
          location_override_city: 'Hong Kong',
          location_override_latitude: 22.3193,
          location_override_longitude: 114.1694,
        },
      ],
    }).agents[0];
    const serviceDraft: ServiceDraft = {
      serviceDate: '',
      serviceUnlimited: false,
      renewalPrice: '',
    };
    const locationDraft: LocationDraft = {
      country: '',
      city: '',
      latitude: '',
      longitude: '',
    };

    syncDraftsFromAgent(agent, serviceDraft, locationDraft);

    expect(serviceDraft).toEqual({
      serviceDate: '2026-12-31',
      serviceUnlimited: true,
      renewalPrice: '$9/mo',
    });
    expect(locationDraft).toEqual({
      country: 'HK',
      city: 'Hong Kong',
      latitude: '22.3193',
      longitude: '114.1694',
    });

    syncDraftsFromAgent(undefined, serviceDraft, locationDraft);
    expect(serviceDraft).toEqual({
      serviceDate: '',
      serviceUnlimited: false,
      renewalPrice: '',
    });
    expect(locationDraft).toEqual({
      country: '',
      city: '',
      latitude: '',
      longitude: '',
    });
  });
});
