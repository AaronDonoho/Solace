// Open shadow roots that hold live content. Each host's root is observed for added and removed hosts;
// removing a host releases it and every nested host, walking children and shadow children once each.

const walk = (root, visit) => {
  const stack = [root];
  while (stack.length) {
    const node = stack.pop();
    if (node.nodeType === 1) visit(node);
    if (node.shadowRoot) stack.push(node.shadowRoot);
    const children = node.children ?? [];
    for (let i = children.length - 1; i >= 0; i--) stack.push(children[i]);
  }
};

export function watch_shadow_hosts({ observe, changed, host }) {
  const hosts = new Map();
  const release = el => {
    const held = hosts.get(el);
    if (!held) return;
    hosts.delete(el);
    held.observer.disconnect();
    held.release?.();
  };
  const unwatch = node => walk(node, release);
  const watch_once = el => {
    const shadow = el.shadowRoot;
    if (!shadow || hosts.has(el)) return;
    const observer = new MutationObserver(records => {
      for (const r of records) {
        for (const n of r.addedNodes) if (n.nodeType === 1) watch(n);
        for (const n of r.removedNodes) if (n.nodeType === 1 && !n.isConnected) unwatch(n);
      }
      changed(records, shadow);
    });
    observer.observe(shadow, observe);
    hosts.set(el, { observer, release: host?.(el, shadow) });
  };
  const watch = node => walk(node, watch_once);
  return { watch, unwatch };
}
