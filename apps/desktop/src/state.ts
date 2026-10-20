import { useCallback, useEffect, useRef, useState } from "react";
import { api, Job, Library, setVideoSupport } from "./api";

export function useLibrary() {
  const [library, setLibrary] = useState<Library | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const load = useCallback(() => {
    api
      .library()
      .then((l) => {
        setVideoSupport(l.host.video);
        setLibrary(l);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    load();
    const un = api.onLibrary(() => {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(load, 250);
    });
    return () => {
      un.then((f) => f());
      window.clearTimeout(timer.current);
    };
  }, [load]);

  return { library, error, reload: load };
}

export interface SpeedHistory {
  samples: number[];
  peak: number;
  started: number;
}

const SAMPLE_MS = 400;
const MAX_SAMPLES = 600;

export function useJobs() {
  const [jobs, setJobs] = useState<Record<string, Job>>({});
  const [history, setHistory] = useState<Record<string, SpeedHistory>>({});
  const lastSample = useRef<Record<string, number>>({});

  useEffect(() => {
    api.jobs().then((list) => setJobs(Object.fromEntries(list.map((j) => [j.projectId, j]))));
    const un = api.onJob((j) => {
      setJobs((all) => ({ ...all, [j.projectId]: j }));
      if (j.phase !== "downloading") return;
      const now = Date.now();
      if (now - (lastSample.current[j.projectId] ?? 0) < SAMPLE_MS) return;
      lastSample.current[j.projectId] = now;
      setHistory((all) => {
        const h = all[j.projectId] ?? { samples: [], peak: 0, started: now };
        let samples = [...h.samples, j.speed];
        if (samples.length > MAX_SAMPLES) samples = samples.filter((_, i) => i % 2 === 0);
        return { ...all, [j.projectId]: { samples, peak: Math.max(h.peak, j.speed), started: h.started } };
      });
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const clear = useCallback((id: string) => {
    api.clearJob(id);
    setJobs((all) => {
      const next = { ...all };
      delete next[id];
      return next;
    });
    setHistory((all) => {
      const next = { ...all };
      delete next[id];
      return next;
    });
  }, []);

  return { jobs, history, clear };
}

export const isActive = (j: Job | undefined) =>
  !!j && (j.phase === "queued" || j.phase === "building" || j.phase === "downloading" || j.phase === "installing" || j.phase === "removing");
