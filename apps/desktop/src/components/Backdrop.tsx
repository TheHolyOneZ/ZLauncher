import { Component, ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { AddEquation, BufferAttribute, BufferGeometry, CustomBlending, OneFactor, OneMinusSrcAlphaFactor, ShaderMaterial, Vector2, Vector3 } from "three";
import { invoke } from "@tauri-apps/api/core";
import { useEffectsLevel } from "../effects";

const Z_PATHS = [
  "M208 448L352 300C385 268 410 258 450 258L640 260C780 262 870 205 972 120L680 405L490 405C445 405 420 418 405 448Z",
  "M888 260L947 260L947 412L352 1003L274 1003L274 866Z",
  "M270 1138L565 855L755 853C790 853 805 838 820 818L1022 818L875 962C845 992 815 1003 770 1003L590 1003C470 1003 370 1060 270 1138Z",
];

const COUNT = 9000;
const DUST = 1400;

function buildField(): BufferGeometry {
  const S = 360;
  const mask = (stroke: boolean) => {
    const c = document.createElement("canvas");
    c.width = c.height = S;
    const ctx = c.getContext("2d");
    if (!ctx) return new Uint8ClampedArray(S * S * 4);
    ctx.scale(S / 1254, S / 1254);
    ctx.translate(12, -2);
    ctx.fillStyle = ctx.strokeStyle = "#fff";
    ctx.lineWidth = 9;
    Z_PATHS.forEach((d) => (stroke ? ctx.stroke(new Path2D(d)) : ctx.fill(new Path2D(d))));
    return ctx.getImageData(0, 0, S, S).data;
  };
  const inside = mask(false);
  const edge = mask(true);
  const total = COUNT + DUST;
  const pos = new Float32Array(total * 3);
  const rnd = new Float32Array(total * 4);
  const kind = new Float32Array(total);
  let seed = 7;
  const rand = () => {
    seed = (seed * 16807) % 2147483647;
    return (seed - 1) / 2147483646;
  };
  const sample = (data: Uint8ClampedArray): [number, number] => {
    for (let tries = 0; tries < 400; tries++) {
      const x = Math.floor(rand() * S);
      const y = Math.floor(rand() * S);
      if (data[(y * S + x) * 4 + 3] > 128) return [x + rand(), y + rand()];
    }
    return [S / 2, S / 2];
  };
  for (let i = 0; i < total; i++) {
    if (i < COUNT) {
      const onEdge = i % 10 < 4;
      const [x, y] = sample(onEdge ? edge : inside);
      pos[i * 3] = (x / S - 0.5) * 2.2;
      pos[i * 3 + 1] = -(y / S - 0.5) * 2.2;
      pos[i * 3 + 2] = (rand() - 0.5) * (onEdge ? 0.06 : 0.22);
    } else {
      const a = rand() * Math.PI * 2;
      const r = 0.9 + rand() * 1.6;
      pos[i * 3] = Math.cos(a) * r * 1.3;
      pos[i * 3 + 1] = Math.sin(a) * r * 0.9;
      pos[i * 3 + 2] = (rand() - 0.5) * 1.8;
      kind[i] = 1;
    }
    for (let k = 0; k < 4; k++) rnd[i * 4 + k] = rand();
  }
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(pos, 3));
  g.setAttribute("aRand", new BufferAttribute(rnd, 4));
  g.setAttribute("aKind", new BufferAttribute(kind, 1));
  return g;
}

const vertex = `
precision highp float;
attribute vec4 aRand;
attribute float aKind;
uniform float uTime, uIntro, uPixel, uAspect, uScale, uPointerOn, uDim;
uniform vec2 uOffset, uPointer, uTilt;
varying float vMix;
varying float vAlpha;
void main() {
  vec3 p = position;
  vec3 cloud = (aRand.xyz - .5) * vec3(4.2, 2.8, 2.4);
  float t = smoothstep(aRand.w * .55, aRand.w * .55 + .45, uIntro);
  p = mix(cloud, p, t);
  float wander = 1. + aKind * 7.;
  p.x += sin(uTime * .6 + aRand.y * 40.) * .007 * wander;
  p.y += cos(uTime * .5 + aRand.x * 40.) * .007 * wander;
  p.z += sin(uTime * .7 + position.x * 2.4 + position.y * 1.3) * .05;
  vec2 d = p.xy - uPointer;
  float push = exp(-dot(d, d) * 8.) * uPointerOn * (1. - aKind * .6);
  p.xy += normalize(d + .0001) * push * .17;
  p.z += push * .35;
  float cy = cos(uTilt.x), sy = sin(uTilt.x);
  p.xz = vec2(p.x * cy - p.z * sy, p.x * sy + p.z * cy);
  float cx = cos(uTilt.y), sx = sin(uTilt.y);
  p.yz = vec2(p.y * cx - p.z * sx, p.y * sx + p.z * cx);
  float persp = 2. / (2. - p.z);
  vec2 s = p.xy * uScale * persp + uOffset;
  gl_Position = vec4(s.x / uAspect, s.y, 0., 1.);
  gl_PointSize = (1. + aRand.z * 1.5 + push * 3.5) * uPixel * persp * mix(1., .75, aKind);
  vMix = clamp(.5 + (position.x - position.y) * .3, 0., 1.);
  float twinkle = .62 + .38 * sin(uTime * (1. + aRand.x * 2.2) + aRand.y * 50.);
  vAlpha = (mix(.9, .3, aKind) * twinkle + push * .4) * t * uDim;
}
`;

const fragment = `
precision highp float;
uniform vec3 uA, uB;
uniform float uLight;
varying float vMix;
varying float vAlpha;
void main() {
  float d = length(gl_PointCoord - .5);
  float a = smoothstep(.5, .05, d) * vAlpha;
  vec3 col = mix(uA, uB, vMix);
  col = mix(col, vec3(1.), smoothstep(.22, 0., d) * .4 * (1. - uLight));
  gl_FragColor = vec4(col * a, a);
}
`;

function rgb(hex: string, into: Vector3): Vector3 {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h.slice(0, 6);
  const n = Number.parseInt(full, 16);
  if (Number.isNaN(n)) return into;
  return into.set(((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255);
}

function fieldMaterial(): ShaderMaterial {
  return new ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uIntro: { value: 0 },
      uPixel: { value: 1.7 },
      uAspect: { value: 1 },
      uScale: { value: 0.6 },
      uPointerOn: { value: 0 },
      uDim: { value: 0.6 },
      uOffset: { value: new Vector2() },
      uPointer: { value: new Vector2(9, 9) },
      uTilt: { value: new Vector2() },
      uA: { value: new Vector3() },
      uB: { value: new Vector3() },
      uLight: { value: 0 },
    },
    vertexShader: vertex,
    fragmentShader: fragment,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    blending: CustomBlending,
    blendEquation: AddEquation,
    blendSrc: OneFactor,
    blendDst: OneFactor,
  });
}

const STILL_TIME = 12;
const devHash = import.meta.env.DEV ? new URLSearchParams(location.hash.slice(1)) : new URLSearchParams();
const DEV_DPR = Number(devHash.get("fxdpr")) || 0;
const DEV_FPS = Number(devHash.get("fxfps")) || 0;

function ZField({ animate }: { animate: boolean }) {
  const { palette, theme } = useEffectsLevel();
  const invalidate = useThree((s) => s.invalidate);
  const size = useThree((s) => s.size);
  const dpr = useThree((s) => s.viewport.dpr);
  const el = useThree((s) => s.gl.domElement);
  const geometry = useMemo(buildField, []);
  const material = useMemo(fieldMaterial, []);
  const state = useRef({ intro: animate ? 0 : 1, time: STILL_TIME, on: 0, want: 0, tx: 0, ty: 0, wtx: 0, wty: 0, px: 9, py: 9 });
  const layout = useRef({ aspect: 1, scale: 0.6, ox: 0, oy: 0 });

  useEffect(
    () => () => {
      geometry.dispose();
      material.dispose();
    },
    [geometry, material],
  );

  useEffect(() => {
    const u = material.uniforms;
    rgb(palette.cyan, u.uA.value);
    rgb(palette.violet, u.uB.value);
    const light = theme === "light";
    u.uLight.value = light ? 1 : 0;
    u.uDim.value = light ? 0.38 : 0.55;
    material.blendDst = light ? OneMinusSrcAlphaFactor : OneFactor;
    material.needsUpdate = true;
    invalidate();
  }, [palette, theme, material, invalidate]);

  useEffect(() => {
    const u = material.uniforms;
    const aspect = size.width / Math.max(size.height, 1);
    const scale = size.width > 900 ? 0.62 : 0.5;
    const l = layout.current;
    l.aspect = aspect;
    l.scale = scale;
    l.ox = aspect * 0.5;
    l.oy = -0.04;
    u.uAspect.value = aspect;
    u.uScale.value = scale;
    u.uOffset.value.set(l.ox, l.oy);
    u.uPixel.value = dpr * Math.max(0.8, Math.min(1.5, size.height / 760)) * 1.6;
    invalidate();
  }, [size.width, size.height, dpr, material, invalidate]);

  useEffect(() => {
    const s = state.current;
    if (!animate) {
      s.intro = 1;
      s.on = 0;
      invalidate();
      return;
    }
    const onMove = (e: PointerEvent) => {
      const b = el.getBoundingClientRect();
      const l = layout.current;
      if (e.clientX < b.left || e.clientY < b.top) {
        s.want = 0;
        return;
      }
      const nx = ((e.clientX - b.left) / b.width) * 2 - 1;
      const ny = -(((e.clientY - b.top) / b.height) * 2 - 1);
      s.px = (nx * l.aspect - l.ox) / l.scale;
      s.py = (ny - l.oy) / l.scale;
      s.want = 1;
      s.wtx = nx * 0.22;
      s.wty = -ny * 0.16;
      if (timer && near()) restart();
    };
    const onLeave = () => {
      s.want = 0;
    };
    let timer = 0;
    let frame = 0;
    let scrolledAt = 0;
    let focused = document.hasFocus();
    const near = () => s.want > 0 && Math.abs(s.px) < 1.5 && Math.abs(s.py) < 1.5;
    const loop = () => {
      timer = 0;
      frame = 0;
      if (document.hidden) return;
      const scrolling = performance.now() - scrolledAt < 220;
      if (!scrolling) invalidate();
      if (!scrolling && focused && !DEV_FPS && (near() || s.on > 0.02)) frame = requestAnimationFrame(loop);
      else timer = window.setTimeout(loop, DEV_FPS ? 1000 / DEV_FPS : scrolling ? 120 : focused ? 1000 / 24 : 1000 / 10);
    };
    const restart = () => {
      window.clearTimeout(timer);
      cancelAnimationFrame(frame);
      loop();
    };
    const onFocus = () => {
      focused = true;
      restart();
    };
    const onBlur = () => {
      focused = false;
    };
    const onScroll = () => {
      scrolledAt = performance.now();
    };
    document.addEventListener("scroll", onScroll, { capture: true, passive: true });
    window.addEventListener("pointermove", onMove, { passive: true });
    document.documentElement.addEventListener("pointerleave", onLeave);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", restart);
    loop();
    return () => {
      window.clearTimeout(timer);
      cancelAnimationFrame(frame);
      document.removeEventListener("scroll", onScroll, { capture: true });
      window.removeEventListener("pointermove", onMove);
      document.documentElement.removeEventListener("pointerleave", onLeave);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", restart);
    };
  }, [animate, el, invalidate]);

  useFrame((_, delta) => {
    const s = state.current;
    const u = material.uniforms;
    const dt = Math.min(0.1, delta);
    if (animate) {
      s.time += dt;
      s.intro = Math.min(1, s.intro + dt / 2.6);
      s.on += (s.want - s.on) * (1 - Math.exp(-dt * 5));
      s.tx += (s.wtx - s.tx) * (1 - Math.exp(-dt * 3));
      s.ty += (s.wty - s.ty) * (1 - Math.exp(-dt * 3));
    }
    u.uTime.value = s.time;
    u.uIntro.value = 1 - Math.pow(1 - s.intro, 3);
    u.uPointerOn.value = s.on;
    u.uPointer.value.set(s.px, s.py);
    u.uTilt.value.set(s.tx + Math.sin(s.time * 0.25) * 0.12, s.ty + Math.cos(s.time * 0.2) * 0.05);
  });

  return <points geometry={geometry} material={material} frustumCulled={false} />;
}

class Fallback extends Component<{ children: ReactNode; onFail: () => void }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(error: unknown) {
    invoke("log", { message: `fx: webgl off (${String(error)})` }).catch(() => {});
    this.props.onFail();
  }
  render() {
    return this.state.failed ? null : this.props.children;
  }
}

export default function Backdrop() {
  const { level } = useEffectsLevel();
  const [shown, setShown] = useState(false);
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    <Fallback onFail={() => setFailed(true)}>
      <Canvas
        className={`fx-canvas ${shown ? "fx-canvas-in" : ""}`}
        dpr={DEV_DPR || Math.min(window.devicePixelRatio || 1, 1.5)}
        flat
        linear
        frameloop="demand"
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", pointerEvents: "none" }}
        gl={{ antialias: false, alpha: true, premultipliedAlpha: true, depth: false, stencil: false, powerPreference: "low-power" }}
        onCreated={({ gl }) => {
          gl.setClearColor(0x000000, 0);
          const ctx = gl.getContext();
          invoke("log", { message: `fx: webgl on ${ctx.getParameter(ctx.RENDERER)}` }).catch(() => {});
          requestAnimationFrame(() => setShown(true));
        }}
        aria-hidden
      >
        <ZField animate={level === "full"} />
      </Canvas>
    </Fallback>
  );
}
