import { afterEach, describe, expect, it } from "vitest";

import { createScrollAnchor, pauseScrollAnchor } from "./scrollAnchor";

/** An element whose viewport top the test sets. */
function placed(top: number) {
  const el = document.createElement("div");
  document.body.appendChild(el);
  const box = { top };
  el.getBoundingClientRect = () =>
    ({ top: box.top, bottom: box.top + 20 }) as DOMRect;
  return { el, box };
}

function setup(candidates: HTMLElement[]) {
  const column = document.createElement("div");
  document.body.appendChild(column);
  column.scrollTop = 0;
  // jsdom does not scroll; keep what was set.
  let scrollTop = 1000;
  Object.defineProperty(column, "scrollTop", {
    get: () => scrollTop,
    set: (v: number) => {
      scrollTop = v;
    },
  });
  let resize: () => void = () => undefined;
  let clock = 1000;
  const anchor = createScrollAnchor(column, {
    pick: () => candidates,
    observe: (cb) => {
      resize = cb;
      return () => undefined;
    },
    now: () => clock,
  });
  return {
    column,
    anchor,
    resize: () => resize(),
    advance: (ms: number) => {
      clock += ms;
    },
    get scrollTop() {
      return scrollTop;
    },
  };
}

afterEach(() => {
  document.body.innerHTML = "";
  pauseScrollAnchor(0, 0);
});

describe("createScrollAnchor", () => {
  it("keeps the content under the top edge in place when content above changes height", () => {
    const a = placed(200);
    const page = setup([a.el]);
    // A widget above was rebuilt 73px shorter: the line moved up.
    a.box.top = 127;
    page.resize();
    expect(page.scrollTop).toBe(927);
  });

  it("ignores a sub-pixel change", () => {
    const a = placed(200);
    const page = setup([a.el]);
    a.box.top = 199.6;
    page.resize();
    expect(page.scrollTop).toBe(1000);
  });

  it("uses the next candidate when the first one is gone (a dismissed finding)", () => {
    const a = placed(200);
    const b = placed(400);
    const page = setup([a.el, b.el]);
    a.el.remove();
    b.box.top = 310;
    page.resize();
    expect(page.scrollTop).toBe(910);
  });

  it("follows the user's own scrolling", () => {
    const a = placed(200);
    const page = setup([a.el]);
    a.box.top = 50;
    page.column.dispatchEvent(new Event("scroll"));
    page.resize();
    expect(page.scrollTop).toBe(1000);
  });

  it("never undoes a scroll made since (CodeMirror's own, a clamp)", () => {
    const a = placed(200);
    const page = setup([a.el]);
    // CodeMirror scrolled the column itself before the resize came in.
    page.column.scrollTop = 9400;
    a.box.top = -8200;
    page.resize();
    expect(page.scrollTop).toBe(9400);
    // From there on it anchors again.
    a.box.top = -8250;
    page.resize();
    expect(page.scrollTop).toBe(9350);
  });

  it("leaves a navigation scroll alone while it runs", () => {
    const a = placed(200);
    const page = setup([a.el]);
    pauseScrollAnchor(500, 1000);
    a.box.top = 100;
    page.resize();
    expect(page.scrollTop).toBe(1000);
    // Once it is over, the anchor is wherever navigation left it.
    page.advance(600);
    a.box.top = 80;
    page.resize();
    expect(page.scrollTop).toBe(980);
  });

  it("stops on destroy", () => {
    const a = placed(200);
    const page = setup([a.el]);
    page.anchor.destroy();
    a.box.top = 100;
    page.resize();
    expect(page.scrollTop).toBe(1000);
  });
});

describe("createScrollAnchor's scroll handling", () => {
  it("notes the anchors once after a scrolled frame, not in every scroll event", () => {
    const column = document.createElement("div");
    document.body.appendChild(column);
    const frames: (() => void)[] = [];
    let picks = 0;
    createScrollAnchor(column, {
      pick: () => {
        picks++;
        return [];
      },
      observe: () => () => undefined,
      now: () => 0,
      afterFrame: (fn) => frames.push(fn),
    });
    const atStart = picks;
    for (let i = 0; i < 5; i++) column.dispatchEvent(new Event("scroll"));
    expect(picks).toBe(atStart);
    expect(frames).toHaveLength(1);
    frames[0]();
    expect(picks).toBe(atStart + 1);
  });
});
