// Keeps the editor's last-line inset matched to the action row drawn over it. A label or a resize can change
// the row, so follow both; a replaced row is observed in place of the old one.
export function watchActionPad(composer) {
  if (!composer) return () => {};
  let resize = null;
  let observed = null;
  const measure = () => {
    const actions = composer.querySelector('.composer-actions');
    if (actions !== observed) {
      resize?.disconnect();
      observed = actions;
      if (actions) resize?.observe(actions);
    }
    if (!actions) {
      composer.style.removeProperty('--md-action-pad');
      return;
    }
    const width = Math.max(actions.getBoundingClientRect().width, actions.scrollWidth);
    composer.style.setProperty('--md-action-pad', `${width}px`);
  };
  resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
  const mutations = typeof MutationObserver === 'undefined' ? null : new MutationObserver(measure);
  mutations?.observe(composer, { childList: true, subtree: true, characterData: true });
  measure();
  return () => {
    resize?.disconnect();
    mutations?.disconnect();
    observed = null;
  };
}
