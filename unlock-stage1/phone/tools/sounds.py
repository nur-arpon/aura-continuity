# Aura phone UI sounds: original, synthesized here (no recorded or third-party assets).
# 22.05 kHz mono 16-bit WAV, each well under 40 KB. Same recipe family as unlock-watch/tools/sounds.py
# so the phone and the watch sound like one product.
#   python tools/sounds.py      -> android/app/src/main/res/raw/aura_*.wav
import math, os, random, struct, wave

SR = 22050
OUT = os.path.join(os.path.dirname(__file__), "..", "android", "app", "src", "main", "res", "raw")


def env(t, a, d):
    return (min(1.0, t / a) if a > 0 else 1.0) * math.exp(-t / d)


def write(name, samples):
    peak = max(1e-9, max(abs(s) for s in samples))
    k = 0.85 / peak
    os.makedirs(OUT, exist_ok=True)
    # 3 ms fade-out so nothing clicks at the end
    n = len(samples); fade = int(SR * 0.003)
    with wave.open(os.path.join(OUT, name + ".wav"), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        frames = []
        for i, s in enumerate(samples):
            g = min(1.0, (n - i) / fade) if fade else 1.0
            frames.append(struct.pack("<h", int(max(-1, min(1, s * k * g)) * 32767)))
        w.writeframes(b"".join(frames))


def tone(freqs, dur, a=0.002, d=0.05, start=0.0, total=None):
    n = int(SR * (total or dur)); out = [0.0] * n
    for f, amp in freqs:
        for i in range(int(SR * start), min(n, int(SR * (start + dur)))):
            t = i / SR - start
            out[i] += amp * math.sin(2 * math.pi * f * t) * env(t, a, d)
    return out


def sweep(f0, f1, dur, a=0.004, d=0.08, start=0.0, total=None, amp=1.0):
    n = int(SR * (total or dur)); out = [0.0] * n; ph = 0.0
    for i in range(int(SR * start), min(n, int(SR * (start + dur)))):
        t = i / SR - start
        f = f0 * (f1 / f0) ** (t / dur)
        ph += 2 * math.pi * f / SR
        out[i] += amp * math.sin(ph) * env(t, a, d)
    return out


def mix(*tracks):
    n = max(len(t) for t in tracks); o = [0.0] * n
    for t in tracks:
        for i, v in enumerate(t): o[i] += v
    return o


random.seed(13)
# tick: a short glassy click (switches, segments, list notches)
write("aura_tick", [s + 0.12 * random.uniform(-1, 1) * math.exp(-i / 40) for i, s in enumerate(tone([(3400, 1), (6800, .25)], .03, d=.006))])
# tap: soft, rounded press
write("aura_tap", tone([(1500, 1), (3000, .2)], .06, d=.014))
# lock: a decisive low thunk + a latch click (the "door closes")
thunk = [0.0] * int(SR * .28)
for i in range(len(thunk)):
    t = i / SR; f = 160 * math.exp(-t * 6) + 68
    thunk[i] = math.sin(2 * math.pi * f * t) * env(t, .002, .075)
latch = tone([(2500, .5), (4200, .2)], .02, d=.004, start=.05, total=.28)
write("aura_lock", mix(thunk, latch))
# unlock: latch click first, then a soft rising open (the door opens)
click = tone([(2700, .55), (4600, .2)], .02, d=.004, total=.42)
rise = sweep(520, 1560, .30, a=.01, d=.16, start=.04, total=.42, amp=.75)
shine = tone([(2093, .35), (3136, .12)], .2, a=.005, d=.09, start=.2, total=.42)
write("aura_unlock", mix(click, rise, shine))
# send (Continue on PC, clipboard / file to PC): an airy upward whoosh
noise = [0.0] * int(SR * .36)
lp = 0.0
for i in range(len(noise)):
    t = i / SR
    lp += 0.18 * (random.uniform(-1, 1) - lp)
    noise[i] = lp * 0.9 * math.sin(math.pi * min(1.0, t / .3)) ** 2
write("aura_send", mix(noise, sweep(700, 2100, .26, a=.03, d=.2, total=.36, amp=.45)))
# arrive (something came from the PC / Continue from laptop): two notes falling into place
write("aura_arrive", mix(tone([(1975.5, 1), (3951, .12)], .3, d=.09, total=.42),
                         tone([(1318.5, 1), (2637, .15)], .3, d=.13, start=.085, total=.42)))
# success (paired, PC unlocked): rising three-note bell chime
notes = [(1318.5, 0.0), (1661.2, 0.09), (1975.5, 0.18)]
write("aura_success", mix(*[tone([(f, 1), (f * 2.76, .16)], .5, d=.16, start=s, total=.75) for f, s in notes]))


def buzz(f, s, total):
    n = int(SR * total); o = [0.0] * n
    for i in range(int(SR * s), min(n, int(SR * (s + .11)))):
        t = i / SR - s
        o[i] = (math.sin(2 * math.pi * f * t) + .35 * math.sin(2 * math.pi * 3 * f * t)) * env(t, .004, .05)
    return o


# error: two low descending soft buzzes
write("aura_error", mix(buzz(330, 0, .3), buzz(247, .14, .3)))
print("wrote", sorted(f for f in os.listdir(OUT) if f.startswith("aura_")))
