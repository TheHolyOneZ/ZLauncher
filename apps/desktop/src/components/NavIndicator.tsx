import { useRef } from "react";
import { gsap, useGSAP, useEffectsLevel } from "../effects";

export function NavIndicator({ active }: { active: string }) {
  const pill = useRef<HTMLDivElement>(null);
  const { motion } = useEffectsLevel();
  const placed = useRef(false);

  useGSAP(
    () => {
      const el = pill.current;
      const box = el?.parentElement;
      if (!box || !el) return;
      const target = box.querySelector<HTMLElement>('[aria-current="true"]');
      if (!target) {
        gsap.to(el, { autoAlpha: 0, duration: motion ? 0.2 : 0 });
        return;
      }
      const to = { y: target.offsetTop, height: target.offsetHeight, autoAlpha: 1 };
      if (!placed.current || !motion) {
        gsap.set(el, to);
        placed.current = true;
        return;
      }
      gsap.to(el, { ...to, duration: 0.55, ease: "expo.out", force3D: true, overwrite: true });
      gsap.fromTo(el, { scaleX: 0.94 }, { scaleX: 1, duration: 0.5, ease: "back.out(2.4)" });
    },
    { dependencies: [active, motion] },
  );

  return <div ref={pill} aria-hidden className="nav-pill" />;
}
