<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';

const props = withDefaults(defineProps<{ labelledBy: string; dismissible?: boolean }>(), {
  dismissible: true,
});
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement | null>(null);
let trigger: HTMLElement | null = null;

function close(): void {
  if (props.dismissible) emit('close');
}

function containTab(event: KeyboardEvent): void {
  if (event.key !== 'Tab' || !dialog.value) return;
  const controls = Array.from(
    dialog.value.querySelectorAll<HTMLElement>(
      'a[href], button, input, select, textarea, summary, [tabindex], [contenteditable]',
    ),
  ).filter(
    (element) =>
      element.tabIndex >= 0 && !element.matches(':disabled') && element.getClientRects().length > 0,
  );
  const first = controls[0];
  const last = controls.at(-1);
  const active = document.activeElement;
  // Native modality blocks background controls, but Tab may otherwise enter browser chrome.
  if (!first || !last) {
    event.preventDefault();
    dialog.value.focus();
  } else if (event.shiftKey && (active === first || active === dialog.value)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (active === last || active === dialog.value)) {
    event.preventDefault();
    first.focus();
  }
}

onMounted(() => {
  trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  // The modal top layer makes background controls inert, including programmatic focus.
  dialog.value?.showModal();
});

onBeforeUnmount(() => {
  dialog.value?.close();
  if (trigger?.isConnected) trigger.focus();
});
</script>

<template>
  <dialog
    ref="dialog"
    class="native-dialog"
    :aria-labelledby="labelledBy"
    aria-modal="true"
    tabindex="-1"
    @cancel.prevent="close"
    @click.self="close"
    @keydown="containTab"
  >
    <slot />
  </dialog>
</template>

<style scoped>
.native-dialog {
  width: 100%;
  height: 100%;
  max-width: none;
  max-height: none;
  margin: 0;
  border: 0;
  box-sizing: border-box;
  color: var(--text-primary);
}
.native-dialog::backdrop {
  background: transparent;
}
</style>
