// motion-lib.js without a browser: the Remotion-compatible maths, the spring against
// an independent RK4 integration, sound cues and what the linter sees of a composition.
// Run: node tests/js/motion-lib.test.js
const fs = require('fs');
const path = require('path');
const src = fs.readFileSync(process.argv[2] || path.join(__dirname, '..', '..', 'assets', 'motion-lib.js'), 'utf8');
global.window = global;
global.innerWidth = 1920; global.innerHeight = 1080;
global.document = { querySelectorAll: () => [], createElement: () => ({ getContext: () => ({}) }) };
global.getComputedStyle = () => ({});
new Function(src)();
const R = global.Remotion, M = global.Motion;
let fails = 0;
const ok = (c, m) => { if (!c) { fails++; console.log('FAIL', m); } else console.log('ok  ', m); };

// spring vs RK4 integration of m x'' = -k (x - 1) - c x'
function rk4(t, { damping: c, mass: m, stiffness: k }) {
  let x = 0, v = 0; const h = 1e-4; const n = Math.round(t / h);
  const a = (x, v) => (-k * (x - 1) - c * v) / m;
  for (let i = 0; i < n; i++) {
    const k1x = v, k1v = a(x, v);
    const k2x = v + h / 2 * k1v, k2v = a(x + h / 2 * k1x, v + h / 2 * k1v);
    const k3x = v + h / 2 * k2v, k3v = a(x + h / 2 * k2x, v + h / 2 * k2v);
    const k4x = v + h * k3v, k4v = a(x + h * k3x, v + h * k3v);
    x += h / 6 * (k1x + 2 * k2x + 2 * k3x + k4x); v += h / 6 * (k1v + 2 * k2v + 2 * k3v + k4v);
  }
  return x;
}
for (const cfg of [{ damping: 10, mass: 1, stiffness: 100 }, { damping: 20, mass: 1, stiffness: 100 }, { damping: 200, mass: 1, stiffness: 100 }, { damping: 12, mass: 0.5, stiffness: 180 }]) {
  let worst = 0;
  for (let f = 0; f <= 60; f += 3) worst = Math.max(worst, Math.abs(R.spring({ frame: f, fps: 30, config: cfg }) - rk4(f / 30, cfg)));
  ok(worst < 1e-6, `spring matches the physics (d${cfg.damping} m${cfg.mass} k${cfg.stiffness}) worst ${worst.toExponential(1)}`);
}
ok(R.spring({ frame: 0, fps: 30 }) === 0, 'spring starts at from');
ok(Math.abs(R.spring({ frame: 200, fps: 30, from: 10, to: 50 }) - 50) < 1e-3, 'spring ends at to');
const d = R.measureSpring({ fps: 30, config: { damping: 200 } });
ok(Math.abs(R.spring({ frame: 20, fps: 30, config: { damping: 200 }, durationInFrames: 20 }) - 1) < 0.006, `durationInFrames stretches the spring to settle on time (natural ${d} frames)`);
ok(R.spring({ frame: 5, fps: 30, delay: 10 }) === 0, 'delay holds the spring at from');
ok(R.spring({ frame: 8, fps: 30, config: { damping: 5, overshootClamping: true } }) <= 1, 'overshootClamping');

// interpolate: Remotion semantics
ok(R.interpolate(15, [0, 30], [0, 100]) === 50, 'interpolate linear');
ok(R.interpolate(45, [0, 30], [0, 100]) === 150, 'extrapolate extend by default');
ok(R.interpolate(45, [0, 30], [0, 100], { extrapolateRight: 'clamp' }) === 100, 'clamp right');
ok(R.interpolate(-5, [0, 30], [0, 100], { extrapolateLeft: 'identity' }) === -5, 'identity left');
ok(Math.abs(R.interpolate(40, [0, 30], [0, 100], { extrapolateRight: 'wrap' }) - 100 / 3) < 1e-9, 'wrap');
ok(R.interpolate(20, [0, 10, 30], [0, 1, 0]) === 0.5, 'multi-point ranges');
ok(Math.abs(R.interpolate(15, [0, 30], [0, 1], { easing: R.Easing.out(R.Easing.cubic) }) - 0.875) < 1e-9, 'Easing.out(cubic)');
ok(Math.abs(R.interpolate(15, [0, 30], [0, 1], { easing: R.Easing.bezier(0.42, 0, 0.58, 1) }) - 0.5) < 1e-4, 'Easing.bezier');
let threw = false; try { R.interpolate(1, [0, 0], [0, 1]); } catch { threw = true; }
ok(threw, 'non-increasing inputRange is refused');
ok(R.interpolateColors(5, [0, 10], ['#000000', '#ffffff']) === 'rgba(128, 128, 128, 1)', 'interpolateColors');
ok(R.random('a') === R.random('a') && R.random('a') !== R.random('b'), 'random is seeded');

// sound cues and the linter view of a composition
M.sfx(0.5, 'whoosh'); M.sfx(0.5, 'whoosh'); M.sfx(1, 'swoosh', { gain: 0.5 });
let bad = false; try { M.sfx(1, 'airhorn'); } catch { bad = true; }
ok(bad, 'unknown sound is refused');
ok(M.beat(4) === 2 && M.beat(2, 90) === 4 / 3, 'beat grid');
global.__motion = null;
M.composition({ durationInFrames: 60, fps: 30, render: (frame) => {
  R.interpolate(frame, [0, 20], [0, 1], { easing: R.Easing.out(R.Easing.exp) });
  R.Sequence({ from: 30, durationInFrames: 30 }, (f) => {
    R.spring({ frame: f, fps: 30, config: { damping: 14 } });
    R.interpolate(f, [5, 15], [0, 1]);
    R.Sfx({ kind: 'pop', from: 5 });
    if (f === 0 || f === 10) ok(R.useCurrentFrame() === f, `Sequence gives local frame ${f}`);
  });
} });
const rep = global.__motion.report();
const kinds = rep.tweens.map((t) => `${t.kind}@${t.at.toFixed(3)}:${t.ease}`);
console.log(kinds.join('  '));
ok(rep.tweens.length === 3, 'one tween per interpolate/spring call site, not per frame');
ok(kinds.includes('interpolate@0.000:out(exp)') && kinds.includes('spring@1.000:spring(d14,m1,k100)') && kinds.includes('interpolate@1.167:linear'), 'tweens carry absolute times and names');
ok(rep.tweens.find((t) => t.ease === 'linear').generic === true, 'linear interpolate counts as generic');
const cues = rep.sfx.map((c) => `${c.kind}@${c.at}`);
console.log(cues.join('  '));
ok(cues.length === 3 && cues.includes('pop@1.16667') && cues.includes('whoosh@0.5') && cues.includes('whoosh@1'), 'cues: deduplicated, absolute, alias resolved');
ok(global.__motion.duration === 2, 'composition duration from frames');
process.exit(fails ? 1 : 0);
