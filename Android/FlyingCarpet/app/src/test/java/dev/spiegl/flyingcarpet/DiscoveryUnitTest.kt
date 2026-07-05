package dev.spiegl.flyingcarpet

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress

class DiscoveryUnitTest {

    // Known-answer test shared with the Rust implementation (core/src/discovery.rs,
    // test_cross_platform_vector) to guarantee the wire formats match.
    @Test
    fun crossPlatformVector() {
        val announcement = DiscoveryAnnouncement(
            role = DiscoveryRole.RECEIVER,
            ipAddress = byteArrayOf(192.toByte(), 168.toByte(), 1, 42),
            timestamp = 1750000000L,
            sequence = 7,
            nonce = ByteArray(32) { 0x11 },
        )
        val key = ByteArray(32) { it.toByte() }
        announcement.sign(key)
        val serialized = announcement.serialize()
        val hex = serialized.joinToString(separator = "") { "%02x".format(it) }
        assertEquals(
            "4643415000010100000000c0a8012a0cda00000000684ee180000000071111111111111111111111111111111111111111111111111111111111111111adc75d44854c84be1627ef8933f16d0fcb26807fccb7562ea3609f13982f7a9a",
            hex
        )
        assertTrue(announcement.verify(key))
    }

    @Test
    fun serializeDeserializeRoundTrip() {
        val localIp = InetAddress.getByAddress(byteArrayOf(10, 0, 0, 5))
        val announcement = DiscoveryAnnouncement.create(DiscoveryRole.SENDER, localIp, 42)
        val key = ByteArray(32) { 0xAB.toByte() }
        announcement.sign(key)

        val deserialized = DiscoveryAnnouncement.deserialize(announcement.serialize())
        assertNotNull(deserialized)
        assertEquals(DiscoveryRole.SENDER, deserialized!!.role)
        assertEquals(42, deserialized.sequence)
        assertArrayEquals(byteArrayOf(10, 0, 0, 5), deserialized.ipAddress)
        assertTrue(deserialized.verify(key))
        assertTrue(!deserialized.verify(ByteArray(32) { 0xCD.toByte() }))
    }

    @Test
    fun unicastScanTargets24() {
        val ip = InetAddress.getByAddress(byteArrayOf(192.toByte(), 168.toByte(), 1, 100))
        val targets = unicastScanTargets(ip, 24)
        assertNotNull(targets)
        assertEquals(253, targets!!.size) // 254 hosts minus our own
        assertTrue(!targets.contains(ip))
    }

    @Test
    fun unicastScanTargetsTooLarge() {
        val ip = InetAddress.getByAddress(byteArrayOf(10, 0, 0, 1))
        // /16 = 65534 hosts, way over MAX_UNICAST_SCAN_HOSTS
        assertEquals(null, unicastScanTargets(ip, 16))
    }
}
