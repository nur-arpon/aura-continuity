package com.spacez.unlock_stage1

import android.app.Activity
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Bundle
import android.os.CancellationSignal

/**
 * FINGERPRINT approval. Native BiometricPrompt (not the local_auth plugin) because the
 * request arrives in a background service, often with the phone locked: this tiny
 * activity shows over the lock screen, and the prompt unlocks the per-use fingerprint key
 * through a CryptoObject, so the answer can only be signed after a real strong-biometric
 * match. No UI of its own.
 */
class ApproveActivity : Activity() {
    private val cancel = CancellationSignal()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        val p = UnlockState.pending.get()
        val mac = UnlockCrypto.fingerprintMacUninit()
        if (p == null || p.level != UnlockCrypto.FINGERPRINT || mac == null) {
            UnlockState.log(if (mac == null) "fingerprint key missing (no strong biometric enrolled?)" else "no fingerprint request pending")
            finish(); return
        }
        BiometricPrompt.Builder(this)
            .setTitle("Unlock your PC?")
            .setSubtitle("Request over ${p.transport}")
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .setNegativeButton("Deny", mainExecutor) { _, _ -> UnlockService.answer(UnlockCrypto.DENY); finish() }
            .build()
            .authenticate(BiometricPrompt.CryptoObject(mac), cancel, mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    UnlockService.answer(UnlockCrypto.FINGERPRINT, result.cryptoObject?.mac)
                    finish()
                }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    UnlockState.log("fingerprint prompt ended: $errString")
                    finish()
                }
            })
    }

    override fun onDestroy() {
        if (!cancel.isCanceled) cancel.cancel()
        super.onDestroy()
    }
}
