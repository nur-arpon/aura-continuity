package com.spacez.unlock_stage1.zerotap

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.ContactsContract
import android.telephony.PhoneNumberUtils
import android.telephony.TelephonyManager

/**
 * WhatsApp chats both ways (item 19): WhatsApp shows only the contact's NAME; the PC needs a
 * number to open the same chat (whatsapp://send?phone=). The name is looked up in this phone's own
 * contacts (READ_CONTACTS, asked once on the zero-tap setup screen), on the phone, in memory only.
 * Groups and unknown names have no number: the PC then just opens WhatsApp.
 */
object Contacts {
    fun granted(c: Context) = c.checkSelfPermission(Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED

    private val cache = object : LinkedHashMap<String, String?>(32, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String?>?) = size > 32
    }

    /** E.164 digits ("61400111222") for a contact's display name, or null. */
    fun numberFor(c: Context, name: String): String? {
        if (name.isBlank() || !granted(c)) return null
        synchronized(cache) { if (cache.containsKey(name)) return cache[name] }
        val iso = runCatching { c.getSystemService(TelephonyManager::class.java).networkCountryIso.uppercase() }.getOrNull()?.ifBlank { null } ?: "AU"
        val n = runCatching {
            c.contentResolver.query(ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER, ContactsContract.CommonDataKinds.Phone.IS_PRIMARY),
                "${ContactsContract.CommonDataKinds.Phone.DISPLAY_NAME} = ?", arrayOf(name),
                "${ContactsContract.CommonDataKinds.Phone.IS_PRIMARY} DESC")?.use { cur ->
                if (cur.moveToFirst()) cur.getString(0) else null
            }
        }.getOrNull()?.let { raw -> e164Digits(raw, iso) ?: PhoneNumberUtils.formatNumberToE164(raw, iso)?.removePrefix("+") }
        synchronized(cache) { cache[name] = n }
        return n
    }

    /** "+61 400 111 222" -> "61400111222"; "0400 111 222" (AU) -> "61400111222". Pure, unit-tested. */
    fun e164Digits(raw: String, iso: String): String? {
        val t = raw.trim()
        val digits = t.filter { it.isDigit() }
        if (digits.length < 6) return null
        if (t.startsWith("+")) return digits
        if (t.startsWith("00")) return digits.removePrefix("00")
        val cc = COUNTRY[iso.uppercase()] ?: return null
        return if (digits.startsWith("0")) cc + digits.drop(1) else cc + digits
    }

    private val COUNTRY = mapOf("AU" to "61", "BD" to "880", "NZ" to "64", "GB" to "44", "US" to "1", "CA" to "1", "IN" to "91")
}
