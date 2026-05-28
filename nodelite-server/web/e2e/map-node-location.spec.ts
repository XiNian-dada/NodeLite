import { test } from '@playwright/test';

// Plan §3.7.2 flow 11: map node location.
// Validation point:
//   - Clicking a node marker on the world map highlights the corresponding
//     node card and navigates to its detail page.
test.fixme('map marker click jumps to node detail', async ({ page }) => {
  await page.goto('/');
  // TODO: locate a known marker (by data-node-id or aria-label), click,
  // assert highlight class on the matching card, then assert URL on follow-up
  // click / direct navigation.
});
