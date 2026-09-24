import { ref } from 'vue';

export const stepUpOpen = ref(false);
let pending: Promise<boolean> | null = null;
let resolvePending: ((confirmed: boolean) => void) | null = null;

export function requestStepUp(): Promise<boolean> {
  if (pending) return pending;
  stepUpOpen.value = true;
  pending = new Promise<boolean>((resolve) => {
    resolvePending = resolve;
  });
  return pending;
}

export function finishStepUp(confirmed: boolean): void {
  stepUpOpen.value = false;
  resolvePending?.(confirmed);
  resolvePending = null;
  pending = null;
}
