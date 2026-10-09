// The page's document handlers. The decisions live in frontend.gleam (blocks_drag, keeps_new_tab), where they are tested.
import { blocks_drag, keeps_new_tab } from '../build/dev/javascript/frontend/frontend.mjs';

export function guard(root) {
  root.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    if (blocks_drag(!!el?.closest?.('input,textarea'))) e.preventDefault();
  });

  // modem's click handler routes every link in-page; let modified clicks keep the browser's new-tab behavior.
  root.addEventListener('click', e => {
    if (keeps_new_tab(!!e.ctrlKey, !!e.metaKey, !!e.shiftKey, !!e.altKey, !!e.target.closest?.('.menu a'))) e.stopImmediatePropagation();
  }, true);
}
