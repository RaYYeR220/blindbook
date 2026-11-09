/**
 * One requestAnimationFrame loop for every below-the-fold scene. A scene only
 * gets frames while its element is on screen (60px margin), and never under
 * prefers-reduced-motion: it is expected to have drawn a still key frame first.
 */

export interface Scene {
  el: Element;
  frame(now: number): void;
  /** called each time the scene scrolls into view */
  onVisible?(): void;
}

const scenes = new Map<Element, Scene>();
const live = new Set<Scene>();
let raf = 0;
let io: IntersectionObserver | null = null;

const reduced = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function tick(now: number) {
  raf = 0;
  for (const s of live) s.frame(now);
  if (live.size) raf = requestAnimationFrame(tick);
}

function kick() {
  if (!raf && live.size && !reduced()) raf = requestAnimationFrame(tick);
}

function observer() {
  io ??= new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        const s = scenes.get(e.target);
        if (!s) continue;
        if (e.isIntersecting) {
          live.add(s);
          s.onVisible?.();
        } else live.delete(s);
      }
      kick();
    },
    { rootMargin: "60px 0px" },
  );
  return io;
}

/** Call `fn` 120 ms after the last resize, and once web fonts have loaded. */
export function onRelayout(fn: () => void): () => void {
  let timer = 0;
  let alive = true;
  const onResize = () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(fn, 120);
  };
  window.addEventListener("resize", onResize);
  document.fonts?.ready.then(() => {
    if (alive) fn();
  });
  return () => {
    alive = false;
    window.clearTimeout(timer);
    window.removeEventListener("resize", onResize);
  };
}

/** Start animating `scene` whenever it is visible. Returns the matching unregister. */
export function register(scene: Scene): () => void {
  if (reduced()) return () => {};
  scenes.set(scene.el, scene);
  observer().observe(scene.el);
  return () => {
    scenes.delete(scene.el);
    live.delete(scene);
    io?.unobserve(scene.el);
    if (!live.size && raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  };
}
