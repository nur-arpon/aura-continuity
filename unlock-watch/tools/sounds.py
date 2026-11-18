# Synthesized UI sounds (original, no assets): 22.05 kHz mono 16-bit WAV, tiny.
import math, struct, wave, random
SR = 22050
def env(t, a, d):  # attack/decay exponential envelope
    return (min(1.0, t / a) if a > 0 else 1.0) * math.exp(-t / d)
def write(name, samples):
    peak = max(1e-9, max(abs(s) for s in samples))
    k = 0.85 / peak
    with wave.open(name + ".wav", "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, s * k)) * 32767)) for s in samples))
def tone(freqs, dur, a=0.002, d=0.05, start=0.0, total=None):
    n = int(SR * (total or dur))
    out = [0.0] * n
    for f, amp in freqs:
        for i in range(int(SR * start), min(n, int(SR * (start + dur)))):
            t = i / SR - start
            out[i] += amp * math.sin(2 * math.pi * f * t) * env(t, a, d)
    return out
def mix(*tracks):
    n = max(len(t) for t in tracks); o = [0.0] * n
    for t in tracks:
        for i, v in enumerate(t): o[i] += v
    return o
random.seed(7)
# tick: very short glassy click
write("cu_tick", [s + 0.15 * random.uniform(-1, 1) * math.exp(-i / 40) for i, s in enumerate(tone([(3200, 1), (6400, .3)], .03, d=.006))])
# tap: soft button press
write("cu_tap", tone([(1400, 1), (2800, .25)], .06, d=.015))
# lock: decisive low "thunk" + latch click
thunk = [0.0] * int(SR * .26)
for i in range(len(thunk)):
    t = i / SR; f = 150 * math.exp(-t * 6) + 70
    thunk[i] = math.sin(2 * math.pi * f * t) * env(t, .002, .07)
latch = tone([(2400, .5), (4100, .2)], .02, d=.004, start=.055, total=.26)
write("cu_lock", mix(thunk, latch))
# success: rising three-note chime (E6 G#6 B6), bell-ish partials
notes = [(1318.5, 0.0), (1661.2, 0.09), (1975.5, 0.18)]
write("cu_success", mix(*[tone([(f, 1), (f * 2.76, .18)], .5, d=.16, start=s, total=.75) for f, s in notes]))
# approved: crisp double tick
write("cu_approved", mix(tone([(2600, 1)], .04, d=.01, total=.16), tone([(3100, 1)], .04, d=.01, start=.08, total=.16)))
# error: two low descending soft buzzes
def buzz(f, s, total):
    n = int(SR * total); o = [0.0] * n
    for i in range(int(SR * s), min(n, int(SR * (s + .11)))):
        t = i / SR - s
        o[i] = (math.sin(2 * math.pi * f * t) + .35 * math.sin(2 * math.pi * 3 * f * t)) * env(t, .004, .05)
    return o
write("cu_error", mix(buzz(330, 0, .3), buzz(247, .14, .3)))
