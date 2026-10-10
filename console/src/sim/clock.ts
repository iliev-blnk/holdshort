export const DAY_MS = 86_400_000;
export type Speed = 30 | 60 | 300 | 600;

/** Replay time in ms since the day start. The app calls frame() from requestAnimationFrame. */
export class Clock {
  #t: number;
  #speed: number;
  #playing = false;
  #lastNow: number | null = null;

  constructor(
    private readonly onTick: (tMs: number) => void,
    startMs = 21_600_000,
    speed: Speed = 60,
  ) {
    this.#t = startMs;
    this.#speed = speed;
  }

  get t(): number {
    return this.#t;
  }
  get playing(): boolean {
    return this.#playing;
  }
  get speed(): number {
    return this.#speed;
  }

  play(): void {
    if (this.#t >= DAY_MS) this.seek(0);
    this.#playing = true;
    this.#lastNow = null;
  }

  pause(): void {
    this.#playing = false;
    this.#lastNow = null;
  }

  toggle(): void {
    if (this.#playing) this.pause();
    else this.play();
  }

  seek(tMs: number): void {
    const t = Math.min(Math.max(tMs, 0), DAY_MS);
    if (t === this.#t) return;
    this.#t = t;
    this.onTick(t);
  }

  setSpeed(x: Speed): void {
    this.#speed = x;
  }

  frame(nowMs: number): void {
    if (!this.#playing) return;
    if (this.#lastNow !== null) {
      this.seek(this.#t + (nowMs - this.#lastNow) * this.#speed);
      if (this.#t >= DAY_MS) this.pause();
    }
    if (this.#playing) this.#lastNow = nowMs;
  }
}
