package com.company.odoonativeprint.core

import java.math.BigInteger
import java.security.MessageDigest

/**
 * Pure-Kotlin RFC 8032 Ed25519 signature verifier.
 * Runs identically on Android API 26..35 and JVM unit tests with zero external crypto deps.
 */
object Ed25519Verifier {
    private val P: BigInteger = BigInteger.ONE.shiftLeft(255).subtract(BigInteger.valueOf(19))
    private val L: BigInteger = BigInteger.ONE.shiftLeft(252)
        .add(BigInteger("27742317777372353535851937790883648493"))
    private val D: BigInteger = BigInteger.valueOf(-121665)
        .multiply(BigInteger.valueOf(121666).modInverse(P))
        .mod(P)
    private val I_CONST: BigInteger = BigInteger.valueOf(2)
        .modPow(P.subtract(BigInteger.ONE).divide(BigInteger.valueOf(4)), P)
    private val BY: BigInteger = BigInteger.valueOf(4)
        .multiply(BigInteger.valueOf(5).modInverse(P))
        .mod(P)
    private val BX: BigInteger = recoverX(BY, false)!!

    private data class Point(val x: BigInteger, val y: BigInteger, val z: BigInteger, val t: BigInteger)

    private val IDENTITY = Point(BigInteger.ZERO, BigInteger.ONE, BigInteger.ONE, BigInteger.ZERO)
    private val BASE = Point(BX, BY, BigInteger.ONE, BX.multiply(BY).mod(P))

    fun verify(publicKey: ByteArray, message: ByteArray, signature: ByteArray): Boolean {
        if (publicKey.size != 32 || signature.size != 64) return false
        val aPoint = decodePoint(publicKey) ?: return false
        val rBytes = signature.copyOfRange(0, 32)
        val sBytes = signature.copyOfRange(32, 64)
        val rPoint = decodePoint(rBytes) ?: return false
        val s = decodeLittleEndian(sBytes)
        if (s < BigInteger.ZERO || s >= L) return false

        val md = MessageDigest.getInstance("SHA-512")
        md.update(rBytes)
        md.update(publicKey)
        md.update(message)
        val h = decodeLittleEndian(md.digest()).mod(L)

        val lhs = scalarMul(BASE, s)
        val rhs = pointAdd(rPoint, scalarMul(aPoint, h))
        return pointEqual(lhs, rhs)
    }

    private fun decodeLittleEndian(bytes: ByteArray): BigInteger {
        val rev = ByteArray(bytes.size + 1)
        for (i in bytes.indices) {
            rev[bytes.size - i] = bytes[i]
        }
        return BigInteger(rev)
    }

    private fun recoverX(y: BigInteger, sign: Boolean): BigInteger? {
        if (y >= P) return null
        val y2 = y.multiply(y).mod(P)
        val num = y2.subtract(BigInteger.ONE).mod(P)
        val den = D.multiply(y2).add(BigInteger.ONE).mod(P)
        val x2 = num.multiply(den.modInverse(P)).mod(P)
        if (x2 == BigInteger.ZERO) {
            return if (sign) null else BigInteger.ZERO
        }
        var x = x2.modPow(P.add(BigInteger.valueOf(3)).divide(BigInteger.valueOf(8)), P)
        if (x.multiply(x).subtract(x2).mod(P) != BigInteger.ZERO) {
            x = x.multiply(I_CONST).mod(P)
        }
        if (x.multiply(x).subtract(x2).mod(P) != BigInteger.ZERO) {
            return null
        }
        if (x.testBit(0) != sign) {
            x = P.subtract(x)
        }
        return x
    }

    private fun decodePoint(bytes: ByteArray): Point? {
        if (bytes.size != 32) return null
        val copy = bytes.copyOf()
        val sign = (copy[31].toInt() and 0x80) != 0
        copy[31] = (copy[31].toInt() and 0x7F).toByte()
        val y = decodeLittleEndian(copy)
        val x = recoverX(y, sign) ?: return null
        return Point(x, y, BigInteger.ONE, x.multiply(y).mod(P))
    }

    private fun pointAdd(p1: Point, p2: Point): Point {
        val a = p1.y.subtract(p1.x).multiply(p2.y.subtract(p2.x)).mod(P)
        val b = p1.y.add(p1.x).multiply(p2.y.add(p2.x)).mod(P)
        val c = p1.t.multiply(BigInteger.valueOf(2)).multiply(D).multiply(p2.t).mod(P)
        val d = p1.z.multiply(BigInteger.valueOf(2)).multiply(p2.z).mod(P)
        val e = b.subtract(a).mod(P)
        val f = d.subtract(c).mod(P)
        val g = d.add(c).mod(P)
        val h = b.add(a).mod(P)
        return Point(
            x = e.multiply(f).mod(P),
            y = g.multiply(h).mod(P),
            z = f.multiply(g).mod(P),
            t = e.multiply(h).mod(P),
        )
    }

    private fun scalarMul(point: Point, scalar: BigInteger): Point {
        var result = IDENTITY
        var addend = point
        var k = scalar
        while (k > BigInteger.ZERO) {
            if (k.testBit(0)) {
                result = pointAdd(result, addend)
            }
            addend = pointAdd(addend, addend)
            k = k.shiftRight(1)
        }
        return result
    }

    private fun pointEqual(p1: Point, p2: Point): Boolean {
        val xEq = p1.x.multiply(p2.z).subtract(p2.x.multiply(p1.z)).mod(P) == BigInteger.ZERO
        val yEq = p1.y.multiply(p2.z).subtract(p2.y.multiply(p1.z)).mod(P) == BigInteger.ZERO
        return xEq && yEq
    }
}
