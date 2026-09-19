// Host for thinking-orbs (MIT, Libraries.dev) without React.
import { resolvePreset, MODE_DRAWS } from "/vendor/thinking-orbs.engine.js";

const LABELS = {
  working: "工作中",
  searching: "搜索中",
  solving: "求解中",
  listening: "聆听中",
  connecting: "连接中",
  weaving: "编织中",
  composing: "撰写中",
  breathing: "思考中",
  shaping: "塑形中",
};

function isDark() {
  const scheme = document.documentElement.getAttribute("data-blora-color-scheme");
  if (scheme === "dark") return true;
  if (scheme === "light") return false;
  return matchMedia("(prefers-color-scheme: dark)").matches;
}

function reducedMotion() {
  return matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function attachThinkingOrb(canvas, options = {}) {
  const size = options.size === 64 ? 64 : 20;
  let orbState = options.state || "working";
  let speedMul = options.speed ?? 1;
  let paused = Boolean(options.paused);
  let dark = isDark();
  let raf = 0;
  let live = false;
  let visible = true;

  canvas.role = "img";
  canvas.style.width = `${size}px`;
  canvas.style.height = `${size}px`;
  canvas.style.display = "block";

  const dpr = Math.min(2, typeof devicePixelRatio === "number" ? devicePixelRatio : 1);
  canvas.width = Math.round(size * dpr);
  canvas.height = Math.round(size * dpr);
  const ctx = canvas.getContext("2d");

  function draw(t) {
    if (!ctx) return;
    const { mode, speed, opts } = resolvePreset(orbState, size);
    const drawMode = MODE_DRAWS[mode];
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);
    drawMode(ctx, size, t, dark, opts);
    canvas.setAttribute("aria-label", LABELS[orbState] || "工作中");
    void speed;
  }

  function tick() {
    const { speed } = resolvePreset(orbState, size);
    draw((performance.now() / 1000) * speed * speedMul);
    if (live) raf = requestAnimationFrame(tick);
  }

  function startLoop() {
    if (paused || reducedMotion() || !visible || document.visibilityState === "hidden") {
      draw(0.6);
      return;
    }
    if (live) return;
    live = true;
    raf = requestAnimationFrame(tick);
  }

  function stopLoop() {
    live = false;
    cancelAnimationFrame(raf);
  }

  const io =
    typeof IntersectionObserver !== "undefined"
      ? new IntersectionObserver(([entry]) => {
          visible = entry.isIntersecting;
          if (visible && document.visibilityState !== "hidden") startLoop();
          else stopLoop();
        })
      : null;
  io?.observe(canvas);

  const onVis = () => {
    if (document.visibilityState === "hidden") stopLoop();
    else if (visible) startLoop();
  };
  document.addEventListener("visibilitychange", onVis);

  const themeObs = new MutationObserver(() => {
    dark = isDark();
    if (!live) draw(0.6);
  });
  themeObs.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-blora-color-scheme"],
  });

  startLoop();

  return {
    setState(next) {
      orbState = next;
    },
    setPaused(next) {
      paused = Boolean(next);
      if (paused) {
        stopLoop();
        draw(0.6);
      } else startLoop();
    },
    destroy() {
      stopLoop();
      io?.disconnect();
      themeObs.disconnect();
      document.removeEventListener("visibilitychange", onVis);
    },
  };
}

window.BloraThinkingOrb = { attachThinkingOrb };
window.dispatchEvent(new Event("blora-orb-ready"));
