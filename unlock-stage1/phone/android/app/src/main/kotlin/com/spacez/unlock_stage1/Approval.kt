package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.watchlink.Verdict

/**
 * The order in which the phone tries to approve one PC challenge. Pure, unit-tested
 * (ApprovalPlanTest); UnlockService.onChallenge walks the list.
 *
 * Whole chain as the owner sees it (PC race + this list), unchanged except for the new WATCH step:
 *   1. watch answers the PC directly (PC's rfcomm-watch leg, its own ECDSA attestation)
 *   2. PHONE_AUTO  - Keystore AUTO key (phone unlocked now or in the last 5 min)
 *   3. WATCH       - NEW 2026-10-20: the paired Galaxy Watch7 says "on wrist + unlocked" in a
 *                    signed reply bound to this challenge's nonce -> approved with no tap
 *                    (owner decision, DESIGN-NO-LOCKOUT.md rule 8 as amended)
 *   4. TEST_TAP    - stage-1 test mode only (expires by itself)
 *   5. NOT_NOW (hands-free race: the PC decides) or PROMPT (fingerprint / tap)
 *
 * WATCH comes after PHONE_AUTO, not before, on purpose: the PC holds a phone AUTO answer to a
 * 400 ms round-trip bound, so waiting up to ~1.2 s for the watch first would make every phone
 * AUTO answer fail that bound whenever the watch is slow or away.
 */
object ApprovalPlan {
    enum class Step { PHONE_AUTO, WATCH, TEST_TAP, NOT_NOW, PROMPT }

    /** How long the phone waits for the watch inside one challenge (the PC's race budget is larger). */
    const val WATCH_TIMEOUT_MS = 1_200L

    /**
     * A watch approval is signed with the TAP-level key: the only approval key the Keystore
     * releases with no user authentication (the AUTO key is exactly what is NOT available when
     * this step runs). The PC accepts TAP >= its AUTO minimum.
     */
    const val WATCH_LEVEL = UnlockCrypto.TAP

    fun steps(mode: String, minLevel: Int, handsFree: Boolean, testMode: Boolean, watchPinned: Boolean): List<Step> {
        val s = ArrayList<Step>()
        if (mode == "auto") {
            if (minLevel <= UnlockCrypto.AUTO) s += Step.PHONE_AUTO
            // never for a FINGERPRINT-level challenge (the PC's escalation asks for a real fingerprint)
            if (minLevel <= WATCH_LEVEL && watchPinned) s += Step.WATCH
        }
        if (minLevel <= UnlockCrypto.TAP && testMode) s += Step.TEST_TAP
        s += if (handsFree) Step.NOT_NOW else Step.PROMPT
        return s
    }

    /** Fail closed: only a fresh, correctly signed, nonce-bound "on wrist + unlocked" reply approves. */
    fun watchApproves(v: Verdict): Boolean = v == Verdict.ALLOW
}
