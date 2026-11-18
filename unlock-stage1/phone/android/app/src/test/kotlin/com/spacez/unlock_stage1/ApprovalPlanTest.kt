package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.ApprovalPlan.Step.*
import com.spacez.unlock_stage1.watchlink.Verdict
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The phone's approval order with WatchGate wired in (owner decision 2026-10-20, rule 8 amended). */
class ApprovalPlanTest {
    private val AUTO_HF = UnlockCrypto.AUTO // base level of the PC's hands-free race challenge

    @Test fun handsFreeRaceTriesPhoneAutoThenWatchThenSaysNotNow() {
        assertEquals(listOf(PHONE_AUTO, WATCH, NOT_NOW), ApprovalPlan.steps("auto", AUTO_HF, handsFree = true, testMode = false, watchPinned = true))
    }

    @Test fun normalChallengeFallsBackToThePromptAfterTheWatch() {
        assertEquals(listOf(PHONE_AUTO, WATCH, PROMPT), ApprovalPlan.steps("auto", UnlockCrypto.AUTO, handsFree = false, testMode = false, watchPinned = true))
    }

    @Test fun noPinnedWatchKeepsTheOldOrder() {
        assertEquals(listOf(PHONE_AUTO, NOT_NOW), ApprovalPlan.steps("auto", AUTO_HF, true, false, watchPinned = false))
        assertEquals(listOf(PHONE_AUTO, TEST_TAP, PROMPT), ApprovalPlan.steps("auto", UnlockCrypto.AUTO, false, testMode = true, watchPinned = false))
    }

    @Test fun watchNeverAnswersAFingerprintChallengeOrOverridesTheUsersMode() {
        // the PC's escalation asks for FINGERPRINT: only the real prompt
        assertEquals(listOf(PROMPT), ApprovalPlan.steps("auto", UnlockCrypto.FINGERPRINT, false, false, watchPinned = true))
        // the user picked tap or fingerprint on the phone: no hands-free at all
        for (m in listOf("tap", "fingerprint")) assertFalse(WATCH in ApprovalPlan.steps(m, UnlockCrypto.AUTO, false, false, true))
    }

    @Test fun watchStepComesAfterPhoneAutoAndBeforeAnyPrompt() {
        val s = ApprovalPlan.steps("auto", UnlockCrypto.AUTO, false, true, true)
        assertTrue(s.indexOf(PHONE_AUTO) < s.indexOf(WATCH))
        assertTrue(s.indexOf(WATCH) < s.indexOf(TEST_TAP))
        assertEquals(PROMPT, s.last())
    }

    @Test fun onlyAllowApprovesFailClosed() {
        for (v in Verdict.entries) assertEquals(v == Verdict.ALLOW, ApprovalPlan.watchApproves(v))
    }

    @Test fun watchApprovalIsSignedAtTapLevelWhichThePcAcceptsForAnAutoChallenge() {
        assertEquals(UnlockCrypto.TAP, ApprovalPlan.WATCH_LEVEL)
        assertTrue(ApprovalPlan.WATCH_LEVEL >= AUTO_HF && ApprovalPlan.WATCH_LEVEL < UnlockCrypto.FINGERPRINT)
    }

    @Test fun lockEventsPreferTheAutoKeyAndFallBackToTheNoAuthKey() {
        assertEquals("a" to "auto key", UnlockCrypto.pickLockEventKey("a", "z"))
        assertEquals("z" to "no-auth key", UnlockCrypto.pickLockEventKey(null, "z"))
        assertEquals(null, UnlockCrypto.pickLockEventKey<String>(null, null))
    }
}
