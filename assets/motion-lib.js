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
  const registry = { timelines: [], allow: {}, meta: {}, cues: [], compMeta: [] };

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
        // sound tied to this move: { sfx: 'whoosh' } or [{ kind, at: 'end' | seconds, gain, each }]
        if (o.sfx) {
          (Array.isArray(o.sfx) ? o.sfx : [o.sfx]).forEach((s) => {
            const spec = typeof s === 'string' ? { kind: s } : s;
            if (i > 0 && !spec.each) return;
            const rel = spec.at === 'end' ? dur : typeof spec.at === 'number' ? spec.at : 0;
            sfx(at + offset + rel, spec.kind, spec);
          });
        }
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

  /* ------------------------------------------------------------------ */
  /* sound: cues that move with the motion                               */
  /* ------------------------------------------------------------------ */
  // Every cue is declared by the scene and synthesised by the renderer
  // (`editemos motion render`), so the sound sits on the exact frame of the
  // motion that caused it: move the tween and its sound moves with it.
  const SFX = ['click', 'tick', 'pop', 'whoosh', 'thud', 'boom', 'riser', 'hit', 'type', 'glitch', 'shimmer', 'file'];
  const cueKeys = new Set();
  function sfx(at, kind, o = {}) {
    if (kind === 'swoosh') kind = 'whoosh';
    if (!SFX.includes(kind)) throw new Error(`Motion.sfx: unknown sound "${kind}". Known: ${SFX.join(', ')} (file needs src)`);
    if (kind === 'file' && !o.src) throw new Error('Motion.sfx: kind "file" needs { src }');
    const t = Math.max(0, +at || 0);
    const key = `${kind}|${t.toFixed(4)}|${o.src || ''}|${o.gain ?? ''}|${o.pitch ?? ''}`;
    if (cueKeys.has(key)) return;
    cueKeys.add(key);
    const cue = { at: +t.toFixed(5), kind, gain: o.gain ?? 1, pitch: o.pitch ?? 1, pan: o.pan ?? 0, seed: o.seed ?? registry.cues.length + 1 };
    if (o.dur !== undefined) cue.dur = o.dur;
    if (o.src) cue.src = String(o.src);
    if (o.trim !== undefined) cue.trim = o.trim;
    registry.cues.push(cue);
  }
  // seconds of beat n on a grid of `bpm` (120 bpm = one beat every 0.5 s)
  const beat = (n, bpm = 120) => (n * 60) / bpm;

  /* ------------------------------------------------------------------ */
  /* Remotion-compatible layer                                           */
  /* ------------------------------------------------------------------ */
  // The same names and the same maths as Remotion (interpolate, spring,
  // interpolateColors, Easing, Sequence, Series, useCurrentFrame ...), without
  // React: a composition is a render(frame) function that writes the DOM or a
  // canvas. Code a model already knows how to write for Remotion runs here with
  // the hooks replaced by plain calls, and stays a pure function of the frame.
  let frames = []; // stack: [absolute, local inside the innermost Sequence, ...]
  let recording = false;
  const usage = new Map();
  let comp = null;

  const named = (fn, name, generic) => Object.assign(fn, { __ename: name, __generic: !!generic });
  const Easing = {
    linear: named((t) => t, 'linear', true),
    ease: named(cubicBezier(0.42, 0, 1, 1), 'ease', true),
    quad: named((t) => t * t, 'quad', true),
    cubic: named((t) => t * t * t, 'cubic', true),
    poly: (n) => named((t) => Math.pow(t, n), `poly(${n})`, n <= 3),
    sin: named((t) => 1 - Math.cos((t * Math.PI) / 2), 'sin', true),
    circle: named((t) => 1 - Math.sqrt(Math.max(0, 1 - t * t)), 'circle', false),
    exp: named((t) => Math.pow(2, 10 * (t - 1)), 'exp', false),
    elastic: (b = 1) => { const p = b * Math.PI; return named((t) => 1 - Math.pow(Math.cos((t * Math.PI) / 2), 3) * Math.cos(t * p), `elastic(${b})`, false); },
    back: (s = 1.70158) => named((t) => t * t * ((s + 1) * t - s), `back(${s})`, false),
    bounce: named((t) => {
      if (t < 1 / 2.75) return 7.5625 * t * t;
      if (t < 2 / 2.75) { const u = t - 1.5 / 2.75; return 7.5625 * u * u + 0.75; }
      if (t < 2.5 / 2.75) { const u = t - 2.25 / 2.75; return 7.5625 * u * u + 0.9375; }
      const u = t - 2.625 / 2.75; return 7.5625 * u * u + 0.984375;
    }, 'bounce', false),
    bezier: (x1, y1, x2, y2) => named(cubicBezier(x1, y1, x2, y2), `bezier(${x1},${y1},${x2},${y2})`, false),
    in: (e) => named((t) => e(t), `in(${e.__ename || 'fn'})`, e.__generic),
    out: (e) => named((t) => 1 - e(1 - t), `out(${e.__ename || 'fn'})`, e.__generic),
    inOut: (e) => named((t) => (t < 0.5 ? e(t * 2) / 2 : 1 - e((1 - t) * 2) / 2), `inOut(${e.__ename || 'fn'})`, e.__generic),
  };

  function note(kind, key, meta) {
    if (!recording || usage.has(key)) return;
    usage.set(key, true);
    registry.compMeta.push(Object.assign({ target: kind, props: [], deltas: {}, count: 1, stagger: 0, kind, loop: false }, meta));
  }
  // the offset of the innermost Sequence in absolute frames
  const offset = () => (frames.length ? frames[0] - frames[frames.length - 1] : 0);
  const isFrame = (v) => frames.length > 0 && frames.includes(v);
  const fpsNow = () => (comp ? comp.fps : 30);

  function interpolateSegment(input, [i0, i1], [o0, o1], easing, left, right) {
    let r = input;
    if (r < i0) {
      if (left === 'identity') return r;
      if (left === 'clamp') r = i0;
      else if (left === 'wrap') { const span = i1 - i0; r = ((((r - i0) % span) + span) % span) + i0; }
    }
    if (r > i1) {
      if (right === 'identity') return r;
      if (right === 'clamp') r = i1;
      else if (right === 'wrap') { const span = i1 - i0; r = ((((r - i0) % span) + span) % span) + i0; }
    }
    if (o0 === o1) return o0;
    if (i1 === i0) return r <= i0 ? o0 : o1;
    return easing((r - i0) / (i1 - i0)) * (o1 - o0) + o0;
  }

  function interpolate(input, inputRange, outputRange, options = {}) {
    if (!Array.isArray(inputRange) || inputRange.length < 2 || inputRange.length !== outputRange.length) {
      throw new Error('interpolate: inputRange and outputRange need the same length (>= 2)');
    }
    for (let i = 1; i < inputRange.length; i++) {
      if (!(inputRange[i] > inputRange[i - 1])) throw new Error(`interpolate: inputRange must be strictly increasing, got [${inputRange.join(', ')}]`);
    }
    let easing = options.easing || Easing.linear;
    if (typeof easing === 'string' || Array.isArray(easing)) {
      // a Motion.ease name or a bezier array works too
      const r = resolveEase(easing);
      easing = named(r.fn, r.name, r.generic);
    }
    const left = options.extrapolateLeft || 'extend';
    const right = options.extrapolateRight || 'extend';
    if (isFrame(input)) {
      const name = easing.__ename || 'custom-fn';
      const fps = fpsNow();
      const start = frames[0] - input + inputRange[0];
      note('interpolate', `i|${start}|${inputRange.join(',')}|${outputRange.join(',')}|${name}`, {
        at: start / fps, dur: (inputRange[inputRange.length - 1] - inputRange[0]) / fps,
        ease: name, custom: !easing.__ename, generic: easing.__ename ? !!easing.__generic : false,
      });
    }
    let i = 1;
    while (i < inputRange.length - 1 && input > inputRange[i]) i++;
    return interpolateSegment(input, [inputRange[i - 1], inputRange[i]], [outputRange[i - 1], outputRange[i]], easing, left, right);
  }

  let colorCtx = null;
  function toRgba(c) {
    const direct = parseColor(c);
    if (direct) return direct;
    if (!colorCtx) colorCtx = document.createElement('canvas').getContext('2d');
    colorCtx.fillStyle = '#000';
    colorCtx.fillStyle = c; // named colours, hsl(): the browser normalises them
    return parseColor(colorCtx.fillStyle) || [0, 0, 0, 1];
  }
  function interpolateColors(input, inputRange, outputRange) {
    const cs = outputRange.map(toRgba);
    const ch = (k) => interpolate(input, inputRange, cs.map((c) => c[k]), { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
    return `rgba(${Math.round(ch(0))}, ${Math.round(ch(1))}, ${Math.round(ch(2))}, ${+ch(3).toFixed(4)})`;
  }

  // The closed form of a damped spring released from rest, at t seconds. Remotion
  // integrates the same equation step by step; stepping an exact solution gives
  // the exact solution, so values agree with Remotion's spring() to rounding.
  function springProgress(t, { damping = 10, mass = 1, stiffness = 100 } = {}) {
    if (t <= 0) return 0;
    const w0 = Math.sqrt(stiffness / mass);
    const z = damping / (2 * Math.sqrt(stiffness * mass));
    const env = Math.exp(-z * w0 * t);
    if (z < 1) {
      const wd = w0 * Math.sqrt(1 - z * z);
      return 1 - env * (Math.cos(wd * t) + ((z * w0) / wd) * Math.sin(wd * t));
    }
    if (z === 1) return 1 - env * (1 + w0 * t);
    // overdamped: two decaying exponentials (the sinh/cosh form overflows on long springs)
    const w1 = w0 * Math.sqrt(z * z - 1);
    const r1 = -z * w0 + w1, r2 = -z * w0 - w1;
    return 1 + (r2 * Math.exp(r1 * t) - r1 * Math.exp(r2 * t)) / (r1 - r2);
  }
  // frames until the spring stays within `threshold` of its target
  function measureSpring({ fps = 30, config = {}, threshold = 0.005 } = {}) {
    let f = 0;
    while (Math.abs(1 - springProgress(f / fps, config)) >= threshold && f < fps * 120) f++;
    let finished = f;
    for (let i = 0; i < 20; i++) {
      f++;
      if (Math.abs(1 - springProgress(f / fps, config)) >= threshold) { i = 0; finished = f + 1; }
    }
    return finished;
  }
  function springFn({ frame, fps, config = {}, from = 0, to = 1, delay = 0, durationInFrames, reverse = false } = {}) {
    if (fps === undefined) fps = fpsNow();
    const cfg = { damping: 10, mass: 1, stiffness: 100, overshootClamping: false, ...config };
    const natural = durationInFrames !== undefined || reverse ? measureSpring({ fps, config: cfg }) : undefined;
    const length = durationInFrames ?? natural;
    if (isFrame(frame)) {
      const start = frames[0] - frame + delay;
      note('spring', `s|${start}|${JSON.stringify(cfg)}|${durationInFrames}|${reverse}`, {
        at: start / fps, dur: (length ?? measureSpring({ fps, config: cfg })) / fps,
        ease: `spring(d${cfg.damping},m${cfg.mass},k${cfg.stiffness})`, custom: true, generic: false,
      });
    }
    let f = frame - delay;
    if (reverse) f = length - f;
    if (durationInFrames !== undefined) f = f / (durationInFrames / natural);
    let p = springProgress(f / fps, cfg);
    if (cfg.overshootClamping) p = Math.min(p, 1);
    return from + (to - from) * p;
  }

  function random(seed) {
    if (seed === null || seed === undefined) throw new Error('random(seed): pass a seed so every render is identical');
    const s = String(seed);
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
    return rng(h >>> 0)();
  }

  const useCurrentFrame = () => (frames.length ? frames[frames.length - 1] : 0);
  const useVideoConfig = () => ({ fps: fpsNow(), durationInFrames: comp ? comp.durationInFrames : 0, width: innerWidth, height: innerHeight, id: comp ? comp.id : 'scene' });

  function show(el, on) {
    resolveTargets(el).forEach((e) => { e.style.display = on ? '' : 'none'; });
  }
  // Remotion's <Sequence from durationInFrames>: children see a frame that starts
  // at 0, and do not exist outside their range (pass `el` to hide that element)
  function Sequence({ from = 0, durationInFrames = Infinity, el } = {}, fn) {
    const local = useCurrentFrame() - from;
    const on = local >= 0 && local < durationInFrames;
    if (el) show(el, on);
    if (on && fn) {
      frames.push(local);
      try { fn(local); } finally { frames.pop(); }
    }
    return on;
  }
  // Remotion's <Series>: sequences back to back; `offset` overlaps (negative) or gaps them
  function Series(items) {
    let at = 0;
    items.forEach((it) => {
      at += it.offset || 0;
      Sequence({ from: at, durationInFrames: it.durationInFrames, el: it.el }, it.render);
      at += it.durationInFrames;
    });
    return at;
  }
  function Freeze({ frame }, fn) {
    frames.push(frame);
    try { fn(frame); } finally { frames.pop(); }
  }
  // sound inside a composition, at a frame of the current Sequence
  function Sfx({ kind, from = 0, ...o }) {
    if (recording) sfx((offset() + from) / fpsNow(), kind, o);
  }
  function Audio({ src, from = 0, volume = 1, startFrom = 0 }) {
    if (recording) sfx((offset() + from) / fpsNow(), 'file', { src, gain: volume, trim: startFrom / fpsNow() });
  }

  // A scene written as render(frame): Remotion's mental model, our renderer.
  function composition(c) {
    if (!c || typeof c.render !== 'function') throw new Error('Motion.composition needs { durationInFrames, fps, render(frame) }');
    if (!(c.durationInFrames > 0)) throw new Error('Motion.composition needs durationInFrames > 0');
    comp = { id: c.id || 'scene', fps: c.fps || 30, durationInFrames: c.durationInFrames };
    const renderAt = (f) => {
      frames = [f];
      try { c.render(f, useVideoConfig()); } finally { frames = []; }
    };
    const tl = {
      duration: () => comp.durationInFrames / comp.fps,
      // a whole frame number when the renderer asks for a frame; fractions only for motion-blur sub-frames
      seek: (t) => renderAt(Math.round(t * comp.fps * 1e6) / 1e6),
      __meta: registry.compMeta,
    };
    return mount(tl, {
      duration: tl.duration(),
      fps: comp.fps,
      // walk every frame once so the linter sees each interpolate/spring and every sound is registered
      beforeReport: () => {
        recording = true;
        try { for (let f = 0; f < comp.durationInFrames; f++) renderAt(f); } finally { recording = false; }
        renderAt(0);
      },
    });
  }

  /* ------------------------------------------------------------------ */
  /* layout check: what a reviewer would catch on a still                */
  /* ------------------------------------------------------------------ */
  function inspect(t) {
    const W = innerWidth, H = innerHeight, safe = 0.05;
    const out = [];
    const items = [];
    const opacityOf = (el) => { let o = 1; for (let e = el; e && e.nodeType === 1; e = e.parentElement) { const cs = getComputedStyle(e); if (cs.display === 'none' || cs.visibility === 'hidden') return 0; o *= parseFloat(cs.opacity); } return o; };
    const bgOf = (el) => { for (let e = el; e && e.nodeType === 1; e = e.parentElement) { const c = parseColor(getComputedStyle(e).backgroundColor); if (c && c[3] > 0.9) return c; } return null; };
    const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
    document.querySelectorAll('body *').forEach((el) => {
      if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE'].includes(el.tagName)) return;
      const own = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
      if (!own) return;
      const op = opacityOf(el);
      if (op < 0.35) return;
      // the text itself, not its box: a centred word in a full-width block is not at the edge
      const range = document.createRange();
      let r = null;
      el.childNodes.forEach((n) => {
        if (n.nodeType !== 3 || !n.textContent.trim()) return;
        range.selectNodeContents(n);
        const b = range.getBoundingClientRect();
        if (b.width < 1) return;
        r = r ? { left: Math.min(r.left, b.left), top: Math.min(r.top, b.top), right: Math.max(r.right, b.right), bottom: Math.max(r.bottom, b.bottom) } : { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
      });
      if (!r) return;
      r.width = r.right - r.left;
      r.height = r.bottom - r.top;
      // a clip-path that hides the text (a mask reveal not started yet) means it is not on screen
      const clip = getComputedStyle(el).clipPath;
      if (clip && /inset\(\s*0(px|%)?\s+100%/.test(clip)) return;
      if (r.width < 2 || r.height < 2) return;
      const cs = getComputedStyle(el);
      const name = (el.id ? '#' + el.id : el.className && typeof el.className === 'string' ? '.' + el.className.split(/\s+/)[0] : el.tagName.toLowerCase()) + ` "${el.textContent.trim().slice(0, 24)}"`;
      items.push({ el, r, name });
      if (r.right < 0 || r.left > W || r.bottom < 0 || r.top > H) return; // parked off screen on purpose
      if (r.left < -2 || r.top < -2 || r.right > W + 2 || r.bottom > H + 2) out.push({ code: 'TEXT_OFFSCREEN', what: name, t });
      else if (r.left < W * safe || r.top < H * safe || r.right > W * (1 - safe) || r.bottom > H * (1 - safe)) out.push({ code: 'TEXT_OUTSIDE_SAFE', what: name, t });
      const px = parseFloat(cs.fontSize);
      if (px && px < H * 0.022 && op > 0.9) out.push({ code: 'TEXT_TINY', what: `${name} at ${Math.round(px)}px`, t });
      const fg = parseColor(cs.color), bg = bgOf(el);
      if (fg && bg) {
        const a = lum(fg), b = lum(bg);
        const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
        if (ratio < 3) out.push({ code: 'LOW_CONTRAST', what: `${name} ${ratio.toFixed(1)}:1`, t });
      }
    });
    for (let i = 0; i < items.length; i++) {
      for (let j = i + 1; j < items.length; j++) {
        const a = items[i], b = items[j];
        if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
        const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
        const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
        if (w <= 0 || h <= 0) continue;
        const small = Math.min(a.r.width * a.r.height, b.r.width * b.r.height);
        if ((w * h) / small > 0.15) out.push({ code: 'TEXT_OVERLAP', what: `${a.name} and ${b.name}`, t });
      }
    }
    return out;
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
      inspect: (t) => inspect(t),
      report: () => {
        if (opts.beforeReport) opts.beforeReport();
        return {
          tweens: registry.timelines.flatMap((tl) => tl.__meta).concat(list.some((tl) => tl.__meta === registry.compMeta) ? registry.compMeta : []),
          allow: registry.allow,
          audit: audit(),
          sfx: registry.cues,
          duration,
        };
      },
    };
    list.forEach((tl) => tl.seek(0));
    return global.__motion;
  }

  const Remotion = {
    interpolate, interpolateColors, spring: springFn, measureSpring, Easing, random,
    useCurrentFrame, useVideoConfig, Sequence, Series, Freeze, Sfx, Audio,
  };
  global.Remotion = Remotion;

  global.Motion = {
    timeline, mount, composition, ease, easeFactories, bezier: cubicBezier, spring, rng, noise, splitText, allow, clamp, lerp, parseColor,
    sfx, beat, remotion: Remotion, inspect,
    // the project's edit brand, injected by `editemos motion render` before the scene runs
    get tokens() { return global.__EDITEMOS_TOKENS__ || {}; },
  };
})(window);
