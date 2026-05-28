import { test } from '@playwright/test';

// Plan §3.7.2 flow 8: chart interaction.
// Validation points (per §3.7.4 we do NOT do pixel diffing):
//   - Hovering a chart shows a tooltip with readable text.
//   - Clicking a chart opens its modal (if applicable) and the modal can close.
//   - Zoom / brush gestures (if present) reflect in tooltip values.
test.fixme('hover surfaces a tooltip', async () => {
  // TODO: navigate to monitor tab, hover the CPU chart, assert a tooltip
  // element appears with text matching /\d+%/ or similar.
});

test.fixme('chart modal opens and closes', async () => {
  // TODO: click chart → modal visible → close → modal gone.
});
