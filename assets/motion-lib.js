/*
 * motion-lib.js: scene-side motion engine for editemos (https://github.com/PentaVM/editemos).
 *
 * No dependencies. Every value on screen is a pure function of the time that
 * `seek(t)` receives, so a scene renders identically on every run and can be
 * captured frame by frame (`editemos motion render`).
 *
 *   const tl = Motion.timeline({ step: 1 / 12 });          // optional: animate on twos
 *   tl.fromTo('.title', { y: 80, opacity: 0 }, { y: 0, opacity: 1 },
 *             { at: 0.12, dur: 0.46, ease: 'slam' });
 *   Motion.mount(tl, { duration: 3, fps: 30 });
 *
 * Eases are named so the linter can tell a deliberate curve from a default.
 * Pass a name from Motion.ease, a [x1,y1,x2,y2] bezier array, or a function.
 */
(function (global) {
  'use strict';

  const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
  const lerp = (a, b, t) => a + (b - a) * t;

  /* ------------------------------------------------------------------ */
  /* easing                                                              */
  /* ------------------------------------------------------------------ */
  function cubicBezier(x1, y1, x2, y2) {
    const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
    const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
    const sx = (t) => ((ax * t + bx) * t + cx) * t;
    const sy = (t) => ((ay * t + by) * t + cy) * t;
    const dx = (t) => (3 * ax * t + 2 * bx) * t + cx;
    return (x) => {
      if (x <= 0) return 0;
      if (x >= 1) return 1;
      let t = x;
      for (let i = 0; i < 8; i++) {
        const e = sx(t) - x;
        if (Math.abs(e) < 1e-6) return sy(t);
        const d = dx(t);
        if (Math.abs(d) < 1e-6) break;
        t -= e / d;
      }
      let lo = 0, hi = 1;
      t = x;
      for (let i = 0; i < 40; i++) {
        const e = sx(t);
        if (Math.abs(e - x) < 1e-7) break;
        if (x > e) lo = t; else hi = t;
        t = (hi + lo) / 2;
      }
      return sy(t);
    };
  }

  // Damped spring in normalised time: f = oscillations over the tween, z = damping ratio.
  function spring(f = 2.2, z = 0.42) {
    const w = 2 * Math.PI * f;
    const wd = w * Math.sqrt(Math.max(1e-6, 1 - z * z));
    return (t) => {
      if (t <= 0) return 0;
      if (t >= 1) return 1;
      const e = Math.exp(-z * w * t);
      const raw = 1 - e * (Math.cos(wd * t) + ((z * w) / wd) * Math.sin(wd * t));
      // pin the end to exactly 1 by blending out the residual over the last 8%
      const k = clamp((t - 0.92) / 0.08);
      return lerp(raw, 1, k * k);
    };
  }

  const power = (n) => ({
    in: (t) => Math.pow(t, n),
    out: (t) => 1 - Math.pow(1 - t, n),
    inOut: (t) => (t < 0.5 ? 0.5 * Math.pow(2 * t, n) : 1 - 0.5 * Math.pow(2 * (1 - t), n)),
  });
  const p4 = power(4);

  // "generic" = what every tool ships as default. The linter counts these.
  const GENERIC = new Set(['linear', 'ease', 'easeIn', 'easeOut', 'easeInOut', 'sineIn', 'sineOut', 'sineInOut', 'quadIn', 'quadOut', 'quadInOut', 'cubicIn', 'cubicOut', 'cubicInOut']);

  const ease = {
    // generic (available, but flagged when they dominate a scene)
    linear: (t) => t,
    ease: cubicBezier(0.25, 0.1, 0.25, 1),
    easeIn: cubicBezier(0.42, 0, 1, 1),
    easeOut: cubicBezier(0, 0, 0.58, 1),
    easeInOut: cubicBezier(0.42, 0, 0.58, 1),
    sineIn: (t) => 1 - Math.cos((t * Math.PI) / 2),
    sineOut: (t) => Math.sin((t * Math.PI) / 2),
    sineInOut: (t) => -(Math.cos(Math.PI * t) - 1) / 2,
    quadIn: power(2).in, quadOut: power(2).out, quadInOut: power(2).inOut,
    cubicIn: power(3).in, cubicOut: power(3).out, cubicInOut: power(3).inOut,
    // signature curves (house vocabulary: extend and rename these in estilo-penta.md)
    slam: cubicBezier(0.16, 1, 0.3, 1),      // expo-like: arrives almost instantly, then lingers
    whip: cubicBezier(0.9, 0, 0.1, 1),       // holds, then goes: for cuts and swipes
    drag: cubicBezier(0.6, 0, 0.2, 1),       // heavy start, long settle
    snapIn: cubicBezier(0.7, 0, 1, 0.5),     // accelerates into a wall
    power4In: p4.in, power4Out: p4.out, power4InOut: p4.inOut,
    expoIn: (t) => (t <= 0 ? 0 : Math.pow(2, 10 * t - 10)),
    expoOut: (t) => (t >= 1 ? 1 : 1 - Math.pow(2, -10 * t)),
    backOut: (t, s = 1.45) => 1 + (s + 1) * Math.pow(t - 1, 3) + s * Math.pow(t - 1, 2),
    backIn: (t, s = 1.45) => (s + 1) * t * t * t - s * t * t,
    spring: spring(2.2, 0.42),
    springTight: spring(3, 0.6),
    springLoose: spring(1.6, 0.3),
    hold: (t) => (t >= 1 ? 1 : 0),            // nothing, then a hard jump on the last frame
  };
  const easeFactories = {
    bezier: cubicBezier,
    spring,
    steps: (n) => (t) => Math.floor(clamp(t) * n) / n,
    // ease up to p, then jump: motion with a snap at the end
    snapAt: (p = 0.8, e = 'power4Out') => (t) => (t >= p ? 1 : resolveEase(e).fn(t / p) * 0.94),
    power,
  };

  function resolveEase(spec) {
    if (typeof spec === 'function') return { fn: spec, name: 'custom-fn', custom: true };
    if (Array.isArray(spec) && spec.length === 4) return { fn: cubicBezier(...spec), name: `bezier(${spec.join(',')})`, custom: true };
    const name = spec || 'ease';
    const fn = ease[name];
    if (!fn) throw new Error(`Motion: unknown ease "${name}". Known: ${Object.keys(ease).join(', ')}`);
    return { fn, name, custom: false, generic: GENERIC.has(name) };
  }

  /* ------------------------------------------------------------------ */
  /* deterministic randomness                                            */
  /* ------------------------------------------------------------------ */
  function rng(seed = 1) {
    let a = seed >>> 0;
    const next = () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    next.range = (lo, hi) => lo + (hi - lo) * next();
    next.int = (lo, hi) => Math.floor(next.range(lo, hi + 1));
    next.pick = (arr) => arr[Math.floor(next() * arr.length)];
    return next;
  }
  // 1-D value noise, seeded: smooth organic jitter that is still a pure function of x
  function noise(seed = 7) {
    const r = rng(seed);
    const table = Array.from({ length: 256 }, () => r() * 2 - 1);
    const at = (i) => table[((i % 256) + 256) % 256];
    return (x) => {
      const i = Math.floor(x), f = x - i, u = f * f * (3 - 2 * f);
      return lerp(at(i), at(i + 1), u);
    };
  }

  /* ------------------------------------------------------------------ */
  /* value parsing                                                       */
  /* ------------------------------------------------------------------ */
  // clipT/R/B/L are inset percentages: the mask-reveal family (clip-path: inset(...))
  const CLIP_KEYS = new Set(['clipT', 'clipR', 'clipB', 'clipL']);
  const TRANSFORM_KEYS = new Set(['x', 'y', 'xPercent', 'yPercent', 'scale', 'scaleX', 'scaleY', 'rotate', 'skewX', 'skewY']);
  const FILTER_KEYS = { blur: 'px', brightness: '', contrast: '', saturate: '', hue: 'deg' };
  const FILTER_FN = { blur: 'blur', brightness: 'brightness', contrast: 'contrast', saturate: 'saturate', hue: 'hue-rotate' };
  const DEFAULTS = { x: 0, y: 0, xPercent: 0, yPercent: 0, scale: 1, scaleX: 1, scaleY: 1, rotate: 0, skewX: 0, skewY: 0, opacity: 1, blur: 0, brightness: 1, contrast: 1, saturate: 1, hue: 0, clipT: 0, clipR: 0, clipB: 0, clipL: 0 };

  function parseColor(s) {
    if (typeof s !== 'string') return null;
    let m = /^#([0-9a-f]{3,8})$/i.exec(s.trim());
    if (m) {
      let h = m[1];
      if (h.length === 3 || h.length === 4) h = h.split('').map((c) => c + c).join('');
      const n = (i) => parseInt(h.slice(i, i + 2), 16);
      return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
    }
    m = /^rgba?\(([^)]+)\)$/i.exec(s.trim());
    if (m) {
      const p = m[1].split(',').map((v) => parseFloat(v));
      return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
    }
    return null;
  }
  function parseValue(v) {
    if (typeof v === 'number') return { n: v, unit: '' };
    const c = parseColor(v);
    if (c) return { color: c };
    const m = /^(-?\d*\.?\d+(?:e[+-]?\d+)?)([a-z%]*)$/i.exec(String(v).trim());
    if (m) return { n: parseFloat(m[1]), unit: m[2] };
    return { str: String(v) };
  }
  function mixValue(a, b, t) {
    if (a.color && b.color) return { color: a.color.map((c, i) => lerp(c, b.color[i], t)) };
    if (a.n !== undefined && b.n !== undefined) return { n: lerp(a.n, b.n, t), unit: b.unit || a.unit };
    return t < 1 ? a : b;
  }
  function fmt(v) {
    if (v.color) return `rgba(${v.color.slice(0, 3).map((c) => Math.round(c)).join(',')},${+v.color[3].toFixed(4)})`;
    if (v.n !== undefined) return `${+v.n.toFixed(4)}${v.unit}`;
    return v.str;
  }

  /* ------------------------------------------------------------------ */
  /* timeline                                                            */
  /* ------------------------------------------------------------------ */
  const registry = { timelines: [], allow: {}, meta: {} };

  function resolveTargets(target) {
    if (!target) return [];
    if (typeof target === 'string') return Array.from(document.querySelectorAll(target));
    if (target instanceof Element) return [target];
    if (Array.isArray(target) || target instanceof NodeList || target instanceof HTMLCollection) return Array.from(target).flatMap(resolveTargets);
    return [];
  }

  function timeline(options = {}) {
    const step = options.step || 0; // seconds; 1/12 = "on twos" at 24 fps, 1/10, 1/8 ...
    const tweens = []; // { el, key, from, to, at, dur, ease, order }
    const meta = []; // for the linter
    const calls = [];
    let order = 0;
    let end = 0;

    function add(target, from, to, o = {}) {
      const els = resolveTargets(target);
      if (!els.length) throw new Error(`Motion: target matched nothing: ${typeof target === 'string' ? target : '[element]'}`);
      const at = o.at ?? 0;
      const dur = Math.max(0, o.dur ?? 0.4);
      const ez = resolveEase(o.ease);
      const stagger = o.stagger || 0; // seconds between targets
      const staggerOrder = o.staggerFrom || 'start'; // start | end | center | edges | random
      const idx = els.map((_, i) => i);
      let ranks = idx;
      if (staggerOrder === 'end') ranks = idx.map((i) => els.length - 1 - i);
      else if (staggerOrder === 'center') ranks = idx.map((i) => Math.abs(i - (els.length - 1) / 2) * 2);
      else if (staggerOrder === 'edges') ranks = idx.map((i) => (els.length - 1) / 2 - Math.abs(i - (els.length - 1) / 2));
      else if (staggerOrder === 'random') { const r = rng(o.seed || 11); ranks = idx.map(() => r() * els.length); }
      const gaps = o.staggerShrink || 1; // each gap multiplied by this (<1 tightens the cascade)
      els.forEach((el, i) => {
        let offset = 0;
        if (stagger) {
          const rank = ranks[i];
          offset = gaps === 1 ? rank * stagger : stagger * ((1 - Math.pow(gaps, rank)) / (1 - gaps));
        }
        const keys = new Set([...Object.keys(from || {}), ...Object.keys(to || {})]);
        keys.forEach((key) => {
          const toV = to && key in to ? parseValue(to[key]) : null;
          const fromV = from && key in from ? parseValue(from[key]) : null;
          tweens.push({ el, key, from: fromV, to: toV, at: at + offset, dur, ez, order: order++ });
        });
        end = Math.max(end, at + offset + dur);
      });
      const propNames = Array.from(new Set([...Object.keys(from || {}), ...Object.keys(to || {})]));
      const deltas = {};
      propNames.forEach((k) => {
        const f = from && k in from ? parseValue(from[k]) : null;
        const t = to && k in to ? parseValue(to[k]) : null;
        const base = DEFAULTS[k] !== undefined ? DEFAULTS[k] : null;
        const fn = f && f.n !== undefined ? f.n : base;
        const tn = t && t.n !== undefined ? t.n : base;
        if (fn !== null && tn !== null) deltas[k] = Math.abs(tn - fn);
      });
      meta.push({
        target: typeof target === 'string' ? target : '[el]',
        props: propNames, deltas,
        at, dur, ease: ez.name, custom: ez.custom, generic: !!ez.generic,
        count: els.length, stagger, kind: o.kind || (from ? 'fromTo' : 'to'), loop: !!o.loop,
      });
      return api;
    }

    const api = {
      fromTo: (target, from, to, o) => add(target, from, to, o),
      from: (target, from, o) => add(target, from, null, { ...o, kind: 'from' }),
      to: (target, to, o) => add(target, null, to, { ...o, kind: 'to' }),
      set: (target, props, at = 0) => add(target, null, props, { at, dur: 0, ease: 'linear', kind: 'set' }),
      // call fn(tNow) on every seek, after tweens: for canvases, counters, procedural scenes
      call: (fn) => { calls.push(fn); return api; },
      duration: () => end,
      seek(t) {
        const tt = step > 0 ? Math.floor(t / step + 1e-9) * step : t;
        // group tweens per (el, key) and evaluate piecewise in start order
        const byEl = new Map();
        for (const tw of tweens) {
          let m = byEl.get(tw.el);
          if (!m) { m = new Map(); byEl.set(tw.el, m); }
          let list = m.get(tw.key);
          if (!list) { list = []; m.set(tw.key, list); }
          list.push(tw);
        }
        byEl.forEach((keys, el) => {
          const state = el.__mstate || (el.__mstate = {});
          keys.forEach((list, key) => {
            list.sort((a, b) => a.at - b.at || a.order - b.order);
            // value before the first tween = its "from" (or the base value / authored value)
            let current = list[0].from || (el.__mbase && el.__mbase[key]) || parseValue(DEFAULTS[key] !== undefined ? DEFAULTS[key] : readBase(el, key));
            for (const tw of list) {
              if (tt < tw.at) break;
              const startV = tw.from || current;
              const endV = tw.to || startV;
              const p = tw.dur === 0 ? 1 : clamp((tt - tw.at) / tw.dur);
              current = mixValue(startV, endV, tw.ez.fn(p));
              if (p < 1) break;
            }
            state[key] = current;
          });
          apply(el, state);
        });
        calls.forEach((fn) => fn(tt));
      },
    };
    api.__meta = meta;
    registry.timelines.push(api);
    return api;
  }

  function readBase(el, key) {
    if (key.startsWith('--')) return parseFloat(getComputedStyle(el).getPropertyValue(key)) || 0;
    const cs = getComputedStyle(el);
    const v = cs[key];
    return v === undefined || v === '' ? 0 : v;
  }

  function apply(el, st) {
    // keep any transform authored in CSS as the base the tweens are composed on top of
    if (el.__mbaseT === undefined) {
      const t = getComputedStyle(el).transform;
      el.__mbaseT = t && t !== 'none' ? t : '';
    }
    const tr = el.__mbaseT ? [el.__mbaseT] : [];
    const num = (k, d = 0) => (st[k] ? st[k].n : d);
    if (st.xPercent || st.yPercent || st.x || st.y) {
      tr.push(`translate(${num('xPercent')}%, ${num('yPercent')}%)`);
      tr.push(`translate(${num('x')}px, ${num('y')}px)`);
    }
    if (st.rotate) tr.push(`rotate(${num('rotate')}deg)`);
    if (st.skewX || st.skewY) tr.push(`skew(${num('skewX')}deg, ${num('skewY')}deg)`);
    if (st.scale || st.scaleX || st.scaleY) {
      const s = st.scale ? st.scale.n : 1;
      tr.push(`scale(${(st.scaleX ? st.scaleX.n : 1) * s}, ${(st.scaleY ? st.scaleY.n : 1) * s})`);
    }
    if (tr.length) el.style.transform = tr.join(' ');
    else if (el.style.transform) el.style.transform = '';
    const filters = [];
    for (const k of Object.keys(FILTER_KEYS)) if (st[k]) filters.push(`${FILTER_FN[k]}(${st[k].n}${FILTER_KEYS[k]})`);
    if (filters.length) el.style.filter = filters.join(' ');
    if (Object.keys(st).some((k) => CLIP_KEYS.has(k))) {
      const c = (k) => (st[k] ? st[k].n : 0);
      el.style.clipPath = `inset(${c('clipT')}% ${c('clipR')}% ${c('clipB')}% ${c('clipL')}%)`;
    }
    for (const k of Object.keys(st)) {
      if (TRANSFORM_KEYS.has(k) || k in FILTER_KEYS || CLIP_KEYS.has(k)) continue;
      const v = st[k];
      if (k.startsWith('--')) el.style.setProperty(k, fmt(v));
      else if (k === 'opacity') el.style.opacity = String(v.n);
      else if (k.startsWith('attr:')) el.setAttribute(k.slice(5), fmt(v));
      else el.style[k] = fmt(v);
    }
  }

  /* ------------------------------------------------------------------ */
  /* helpers for authors                                                 */
  /* ------------------------------------------------------------------ */
  function splitText(target, mode = 'chars') {
    const el = typeof target === 'string' ? document.querySelector(target) : target;
    const text = el.textContent;
    el.setAttribute('aria-label', text);
    el.textContent = '';
    const out = [];
    const frag = document.createDocumentFragment();
    const mk = (s, cls) => {
      const span = document.createElement('span');
      span.className = cls;
      span.style.display = 'inline-block';
      span.style.whiteSpace = 'pre';
      span.textContent = s;
      return span;
    };
    if (mode === 'words') {
      text.split(/(\s+)/).forEach((w) => {
        if (!w) return;
        if (/^\s+$/.test(w)) frag.appendChild(document.createTextNode(w));
        else { const s = mk(w, 'mw'); frag.appendChild(s); out.push(s); }
      });
    } else {
      Array.from(text).forEach((ch) => {
        if (/\s/.test(ch)) frag.appendChild(document.createTextNode(ch));
        else { const s = mk(ch, 'mc'); frag.appendChild(s); out.push(s); }
      });
    }
    el.appendChild(frag);
    return out;
  }

  function allow(code, reason) {
    if (!reason || String(reason).length < 8) throw new Error('Motion.allow needs a real reason (>= 8 chars)');
    registry.allow[code] = reason;
  }

  function audit() {
    const fonts = new Map();
    document.querySelectorAll('body *').forEach((el) => {
      if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE'].includes(el.tagName)) return;
      const own = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
      if (!own) return;
      const cs = getComputedStyle(el);
      const fam = cs.fontFamily.split(',')[0].replace(/["']/g, '').trim();
      fonts.set(fam, (fonts.get(fam) || 0) + 1);
    });
    return { fonts: Object.fromEntries(fonts) };
  }

  function mount(tls, opts = {}) {
    const list = Array.isArray(tls) ? tls : [tls];
    const duration = opts.duration ?? Math.max(...list.map((t) => t.duration()));
    global.__motion = {
      duration,
      fps: opts.fps || 30,
      seek: async (t) => {
        list.forEach((tl) => tl.seek(t));
        if (opts.onSeek) opts.onSeek(t);
      },
      report: () => ({
        tweens: registry.timelines.flatMap((tl) => tl.__meta),
        allow: registry.allow,
        audit: audit(),
        duration,
      }),
    };
    list.forEach((tl) => tl.seek(0));
    return global.__motion;
  }

  global.Motion = {
    timeline, mount, ease, easeFactories, bezier: cubicBezier, spring, rng, noise, splitText, allow, clamp, lerp, parseColor,
    // the project's edit brand, injected by `editemos motion render` before the scene runs
    get tokens() { return global.__EDITEMOS_TOKENS__ || {}; },
  };
})(window);
