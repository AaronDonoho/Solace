import { main } from '../build/dev/javascript/frontend/frontend.mjs';

document.addEventListener('dragstart', e => {
  const el = e.composedPath?.()[0] || e.target;
  if (!el?.closest?.('input,textarea')) e.preventDefault();
});

// modem's click handler routes every link in-page; let modified clicks keep the browser's new-tab behavior.
document.addEventListener('click', e => {
  if ((e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) && e.target.closest?.('.menu a')) e.stopImmediatePropagation();
}, true);

main();
