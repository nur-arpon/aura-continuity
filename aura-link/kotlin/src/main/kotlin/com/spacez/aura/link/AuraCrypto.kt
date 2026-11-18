package com.spacez.aura.link

import java.io.DataInputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.PublicKey
import java.security.SecureRandom
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPrivateKeySpec
import java.security.spec.ECPublicKeySpec
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Aura Link v1 primitives, mirror of aura-link/rust (handshake.rs, aead.rs, wire.rs).
 * Only the platform's standard JCA: HmacSHA256, SHA-256, EC P-256 (secp256r1) + ECDH,
 * AES/GCM/NoPadding. HKDF is RFC 5869 over HmacSHA256 (checked against the RFC's test case 1
 * and the shared vectors in aura-link/test-vectors).
 */
class AuraException(msg: String) : IOException(msg)

object Wire {
    const val MAX_RECORD = 65535
    const val R_HELLO: Byte = 0x01
    const val R_AUTH: Byte = 0x02
    const val R_DATA: Byte = 0x03
    const val R_ERROR: Byte = 0x04
    const val R_FAST_HELLO: Byte = 0x10
    const val R_FAST_OK: Byte = 0x11

    fun write(out: OutputStream, rec: ByteArray) {
        if (rec.isEmpty() || rec.size > MAX_RECORD) throw AuraException("record length ${rec.size}")
        val b = ByteArray(rec.size + 2)
        b[0] = (rec.size ushr 8).toByte(); b[1] = rec.size.toByte()
        System.arraycopy(rec, 0, b, 2, rec.size)
        out.write(b); out.flush()
    }

    fun read(ins: InputStream): ByteArray {
        val d = if (ins is DataInputStream) ins else DataInputStream(ins)
        val n = d.readUnsignedShort()
        if (n == 0) throw AuraException("empty record")
        return ByteArray(n).also { d.readFully(it) }
    }
}

/** Big-endian reader that throws AuraException (never IndexOutOfBounds) on short input. */
class Cur(private val b: ByteArray) {
    var p = 0
    fun take(n: Int): ByteArray {
        if (n < 0 || p + n > b.size) throw AuraException("short input: need $n at $p of ${b.size}")
        return b.copyOfRange(p, p + n).also { p += n }
    }
    fun u8(): Int = take(1)[0].toInt() and 0xff
    fun u16(): Int = take(2).let { ((it[0].toInt() and 0xff) shl 8) or (it[1].toInt() and 0xff) }
    fun u32(): Int = take(4).fold(0) { a, x -> (a shl 8) or (x.toInt() and 0xff) }
    fun u64(): Long = take(8).fold(0L) { a, x -> (a shl 8) or (x.toLong() and 0xff) }
    fun rest(): ByteArray = take(b.size - p)
}

fun be16(v: Int) = byteArrayOf((v ushr 8).toByte(), v.toByte())
fun be32(v: Int) = byteArrayOf((v ushr 24).toByte(), (v ushr 16).toByte(), (v ushr 8).toByte(), v.toByte())
fun be64(v: Long) = ByteArray(8) { (v ushr (56 - 8 * it)).toByte() }
fun ByteArray.hex() = joinToString("") { "%02x".format(it) }
fun String.unhex(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()

object AuraCrypto {
    val PSK_LABEL = "aura-link/v1/psk".toByteArray()
    private val rng = SecureRandom()

    fun random(n: Int) = ByteArray(n).also { rng.nextBytes(it) }

    fun hmac(key: ByteArray, vararg parts: ByteArray): ByteArray {
        val m = Mac.getInstance("HmacSHA256")
        m.init(SecretKeySpec(key, "HmacSHA256"))
        parts.forEach { m.update(it) }
        return m.doFinal()
    }

    fun sha256(vararg parts: ByteArray): ByteArray {
        val d = MessageDigest.getInstance("SHA-256")
        parts.forEach { d.update(it) }
        return d.digest()
    }

    /** RFC 5869 */
    fun hkdfExtract(salt: ByteArray, ikm: ByteArray) = hmac(salt, ikm)

    /** RFC 5869, any length up to 255 * 32 */
    fun hkdfExpand(prk: ByteArray, info: ByteArray, len: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        var t = ByteArray(0)
        var i = 1
        while (out.size() < len) {
            t = hmac(prk, t, info, byteArrayOf(i.toByte()))
            out.write(t); i++
        }
        return out.toByteArray().copyOf(len)
    }

    class Keys(val auth: ByteArray, val i2r: ByteArray, val r2i: ByteArray, val fast: ByteArray)

    fun schedule(salt: ByteArray, z: ByteArray, th: ByteArray): Keys {
        val prk = hkdfExtract(salt, z)
        fun x(l: String) = hkdfExpand(prk, l.toByteArray() + th, 32)
        return Keys(x("aura auth"), x("aura i2r"), x("aura r2i"), x("aura fast"))
    }

    fun transcript(hi: ByteArray, hr: ByteArray) = sha256("AURA-LINK/1".toByteArray(), be16(hi.size), hi, be16(hr.size), hr)

    fun pskFromK0(k0: ByteArray) = hmac(k0, PSK_LABEL)

    /** The phone: PSK through the Keystore's no-auth level-0 HMAC key (raw key never leaves it). */
    fun pskFromMac(k0Mac: Mac): ByteArray = k0Mac.doFinal(PSK_LABEL)

    fun pskProof(kAuth: ByteArray, senderRole: Int) = hmac(kAuth, "AURA auth".toByteArray(), byteArrayOf(senderRole.toByte()))

    fun watchAuthMessage(th: ByteArray) = "AURA watch auth".toByteArray() + th

    fun fastKeys(kFast: ByteArray, ticket: ByteArray): Pair<ByteArray, ByteArray> =
        hkdfExpand(kFast, "fast c2s".toByteArray() + ticket, 32) to hkdfExpand(kFast, "fast s2c".toByteArray() + ticket, 32)

    fun ctEq(a: ByteArray, b: ByteArray) = MessageDigest.isEqual(a, b)
}

/** One-session P-256 key pair (platform EC provider). */
class Ephemeral private constructor(private val priv: PrivateKey, val public: ByteArray) {
    companion object {
        private val P = BigInteger("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff", 16)
        private val B = BigInteger("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b", 16)
        val params: ECParameterSpec by lazy {
            (KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair().public as ECPublicKey).params
        }

        fun generate(): Ephemeral {
            val g = KeyPairGenerator.getInstance("EC")
            g.initialize(ECGenParameterSpec("secp256r1"))
            val kp = g.generateKeyPair()
            return Ephemeral(kp.private, encode(kp.public as ECPublicKey))
        }

        /** Test vectors only: fixed scalar plus its known public point. */
        fun fromParts(scalar: ByteArray, pub: ByteArray): Ephemeral =
            Ephemeral(KeyFactory.getInstance("EC").generatePrivate(ECPrivateKeySpec(BigInteger(1, scalar), params)), pub)

        private fun fixed32(v: BigInteger): ByteArray {
            val b = v.toByteArray()
            return when {
                b.size == 32 -> b
                b.size > 32 -> b.copyOfRange(b.size - 32, b.size)
                else -> ByteArray(32 - b.size) + b
            }
        }

        fun encode(k: ECPublicKey): ByteArray = byteArrayOf(4) + fixed32(k.w.affineX) + fixed32(k.w.affineY)

        /** SEC1 uncompressed point -> key; refuses points that are not on P-256. */
        fun decode(b: ByteArray): PublicKey {
            if (b.size != 65 || b[0] != 4.toByte()) throw AuraException("peer key is not an uncompressed P-256 point")
            val x = BigInteger(1, b.copyOfRange(1, 33)); val y = BigInteger(1, b.copyOfRange(33, 65))
            if (x >= P || y >= P) throw AuraException("peer key out of range")
            val lhs = y.modPow(BigInteger.valueOf(2), P)
            val rhs = x.modPow(BigInteger.valueOf(3), P).subtract(x.multiply(BigInteger.valueOf(3))).add(B).mod(P)
            if (lhs != rhs) throw AuraException("peer key is not a P-256 point")
            return KeyFactory.getInstance("EC").generatePublic(ECPublicKeySpec(ECPoint(x, y), params))
        }
    }

    fun agree(peer: ByteArray): ByteArray {
        val ka = KeyAgreement.getInstance("ECDH")
        ka.init(priv)
        ka.doPhase(decode(peer), true)
        return ka.generateSecret()
    }
}

/** AES-256-GCM records, nonce = 0u32 || counter u64, aad = 0x03. */
class Sealer(private val key: ByteArray) {
    private var ctr = 0L
    fun seal(pt: ByteArray): ByteArray {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, ByteArray(4) + be64(ctr)))
        c.updateAAD(byteArrayOf(Wire.R_DATA))
        ctr++
        return byteArrayOf(Wire.R_DATA) + c.doFinal(pt)
    }
}

class Opener(private val key: ByteArray) {
    private var ctr = 0L
    fun open(rec: ByteArray): ByteArray {
        if (rec.size < 17 || rec[0] != Wire.R_DATA) throw AuraException("not a DATA record")
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, ByteArray(4) + be64(ctr)))
        c.updateAAD(byteArrayOf(Wire.R_DATA))
        val pt = runCatching { c.doFinal(rec, 1, rec.size - 1) }.getOrElse { throw AuraException("record failed authentication") }
        ctr++
        return pt
    }
}
