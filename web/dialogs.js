/* Application-owned stack; controls and surfaces use published Blora Design tokens. */
(() => {
  "use strict";
  const stack = [];
  const closing = new Set();
  const host = document.createElement("div");
  host.id = "blora-window-stack";
  document.body.append(host);
  const focusable = panel => [...panel.querySelectorAll('button, input, select, textarea, a[href], [tabindex="0"]')]
    .filter(el => !el.disabled && !el.closest('[hidden], [inert]') && el.getClientRects().length);
  const background = new Map();
  let sequence = 0;
  const top = () => [...closing].at(-1) || stack.at(-1);
  function focusWindow(win) {
    (win.lastFocus?.isConnected && !win.lastFocus.closest('[hidden]') ? win.lastFocus : focusable(win.panel)[0] || win.panel).focus();
  }
  function sync() {
    if (stack.length || closing.size) {
      for (const node of document.body.children) {
        if (node === host || node.tagName === "SCRIPT") continue;
        if (!background.has(node)) background.set(node, { inert: node.inert, dim: node.style.getPropertyValue('--ba-window-dim') });
        node.inert = true;
        node.style.setProperty('--ba-window-dim', Math.max(.35, 1 - stack.length * .16));
        node.classList.add('ba-window-background');
      }
    } else {
      for (const [node, previous] of background) {
        node.inert = previous.inert;
        node.style.setProperty('--ba-window-dim', previous.dim);
        node.classList.remove('ba-window-background');
      }
      background.clear();
    }
    host.hidden = !stack.length && !closing.size;
    host.classList.toggle('ba-window-stack--closing', !stack.length && closing.size > 0);
    stack.forEach((win, index) => {
      const active = win === top();
      win.layer.inert = !active;
      win.panel.setAttribute('aria-modal', String(active));
      win.layer.style.zIndex = index + 1;
      win.panel.style.setProperty('--ba-window-dim', Math.max(.4, 1 - (stack.length - 1 - index) * .18));
      win.layer.classList.toggle('ba-window-layer--active', active);
    });
  }
  function close(win = top(), result = false) {
    const index = stack.indexOf(win);
    if (index < 0 || win.closed) return win?.finished;
    for (const child of stack.slice(index + 1).reverse()) close(child, false);
    stack.splice(index, 1);
    win.closed = true;
    win.controller.abort();
    closing.add(win);
    win.layer.inert = true;
    win.layer.classList.add('ba-window-layer--closing');
    win.panel.setAttribute('aria-busy', 'true');
    sync();
    win.finished = new Promise(resolve => {
      let done = false;
      const finish = () => {
        if (done) return; done = true;
        clearTimeout(timer); win.layer.remove(); closing.delete(win);
        try { win.onClose?.(result); } finally {
          win.resolve(result); sync();
          if (!closing.size) {
            if (top()) focusWindow(top());
            else if (win.origin?.isConnected && !win.origin.closest('[inert]')) win.origin.focus();
          }
          resolve(result);
        }
      };
      const duration = matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 180;
      const timer = setTimeout(finish, duration + 30);
      win.panel.addEventListener('transitionend', event => { if (event.target === win.panel && event.propertyName === 'opacity') finish(); });
      if (!duration) queueMicrotask(finish);
    });
    return win.finished;
  }
  function open(name, { title = name, parent = top(), onClose } = {}) {
    const existing = stack.find(win => win.name === name);
    if (existing) {
      while (stack.at(-1) !== existing) close(stack.at(-1));
      existing.minimized = false;
      existing.body.hidden = false;
      existing.layer.classList.remove('ba-window-layer--minimized');
      existing.minimize.textContent = '最小化';
      focusWindow(existing);
      return existing;
    }
    const win = { name, parent, origin: document.activeElement, controller: new AbortController(), closed: false, onClose };
    win.result = new Promise(resolve => { win.resolve = resolve; });
    win.layer = document.createElement('div');
    win.layer.className = 'ba-window-layer';
    win.panel = document.createElement('section');
    win.panel.className = 'ba-window';
    win.panel.tabIndex = -1;
    win.panel.setAttribute('role', 'dialog');
    const heading = document.createElement('h2');
    heading.id = `ba-window-title-${++sequence}`;
    heading.textContent = title;
    win.panel.setAttribute('aria-labelledby', heading.id);
    const header = document.createElement('header');
    header.className = 'ba-window__header';
    const controls = document.createElement('div');
    controls.className = 'ba-window__controls';
    function control(text, action) {
      const button = document.createElement('button');
      button.type = 'button'; button.className = 'blora-button';
      button.dataset.variant = 'ghost'; button.dataset.size = 'sm'; button.textContent = text;
      button.addEventListener('click', action); controls.append(button); return button;
    }
    win.minimize = control('最小化', () => {
      win.minimized = !win.minimized;
      win.body.hidden = win.minimized;
      win.layer.classList.toggle('ba-window-layer--minimized', win.minimized);
      win.minimize.textContent = win.minimized ? '恢复' : '最小化';
      win.minimize.focus();
    });
    control('展开', event => {
      win.minimized = false; win.body.hidden = false; win.layer.classList.remove('ba-window-layer--minimized'); win.minimize.textContent = '最小化';
      const expanded = win.panel.classList.toggle('ba-window--expanded');
      event.currentTarget.textContent = expanded ? '收起' : '展开';
      event.currentTarget.setAttribute('aria-pressed', String(expanded));
    });
    control('关闭', () => close(win));
    header.append(heading, controls);
    win.body = document.createElement('div'); win.body.className = 'ba-window__body';
    win.panel.append(header, win.body); win.layer.append(win.panel); host.append(win.layer);
    win.panel.addEventListener('focusin', event => { win.lastFocus = event.target; });
    win.close = result => close(win, result);
    stack.push(win); sync(); queueMicrotask(() => { if (top() === win) focusWindow(win); });
    return win;
  }
  document.addEventListener('keydown', event => {
    const win = top(); if (!win) return;
    if (closing.size) { event.preventDefault(); event.stopImmediatePropagation(); return; }
    if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); close(win); }
    else if (win.minimized && event.key === 'Enter') { event.preventDefault(); event.stopImmediatePropagation(); win.minimize.click(); }
    else if (event.key === 'Tab') {
      const items = focusable(win.panel); const index = items.indexOf(document.activeElement);
      event.preventDefault(); event.stopImmediatePropagation();
      (items.length ? items[(index + (event.shiftKey ? -1 : 1) + items.length) % items.length] : win.panel).focus();
    } else if (!win.panel.contains(event.target)) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  host.addEventListener('keydown', event => event.stopPropagation());
  document.addEventListener('focusin', event => {
    if (!closing.size && top() && !top().panel.contains(event.target)) focusWindow(top());
  }, true);
  for (const type of ['click', 'pointerdown', 'wheel', 'paste']) document.addEventListener(type, event => {
    if (top() && (closing.size || !top().panel.contains(event.target))) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, { capture: true, passive: false });
  host.hidden = true;
  window.BloraDialogs = Object.freeze({ open, close, top, get depth() { return stack.length; } });
})();
