// Tests for gmsky_protocol.h. Build and run: see protocol/CMakeLists.txt.
// The layout itself is checked at compile time by the static_asserts in the header.
#include "gmsky_protocol.h"

#include <cmath>
#include <cstdio>
#include <cstring>

static int g_failures = 0;

#define CHECK(c)                                                         \
    do {                                                                 \
        if (!(c)) {                                                      \
            std::printf("FAIL %s:%d  %s\n", __FILE__, __LINE__, #c);     \
            ++g_failures;                                                \
        }                                                                \
    } while (0)

static bool Near(double a, double b, double eps = 1e-9) { return std::fabs(a - b) < eps; }

static void TestSlot() {
    GmskyPlayerStateSlot slot;
    std::memset(&slot, 0, sizeof(slot));
    GmskyPlayerState out{};
    CHECK(!gmsky::SlotRead(slot, out));  // never written

    GmskyPlayerState in{};
    in.tick = 42;
    in.pos[0] = 1.5;
    in.yaw_deg = 90.0f;
    gmsky::SlotWrite(slot, in);
    uint32_t seq = 0;
    CHECK(gmsky::SlotRead(slot, out, &seq));
    CHECK(seq == 2);
    CHECK(out.tick == 42 && out.pos[0] == 1.5 && out.yaw_deg == 90.0f);
}

static void TestPosition() {
    const double origin[3] = {10000.0, -20000.0, 3000.0};  // Skyrim
    const double anchor[3] = {0.0, 0.0, -12000.0};         // Garry's Mod
    // Standing on the anchor = standing on the origin.
    auto s = gmsky::GmodToSkyrim(anchor, anchor, origin);
    CHECK(Near(s.x, origin[0]) && Near(s.y, origin[1]) && Near(s.z, origin[2]));
    // 64 Garry's Mod units (eye height) = 120.5 Skyrim units (first-person camera height).
    const double g[3] = {64.0, 0.0, -12000.0 + 64.0};
    s = gmsky::GmodToSkyrim(g, anchor, origin);
    CHECK(Near(s.x, origin[0] + 120.5) && Near(s.z, origin[2] + 120.5));
    // Round trip.
    const double sv[3] = {s.x, s.y, s.z};
    auto back = gmsky::SkyrimToGmod(sv, anchor, origin);
    CHECK(Near(back.x, g[0]) && Near(back.y, g[1]) && Near(back.z, g[2]));
}

static void TestAngles() {
    const double d2r = gmsky::kPi / 180.0;
    CHECK(Near(gmsky::GmodYawToSkyrimHeading(90.0), 0.0));          // north
    CHECK(Near(gmsky::GmodYawToSkyrimHeading(0.0), 90.0 * d2r));    // east
    CHECK(Near(gmsky::GmodYawToSkyrimHeading(180.0), 270.0 * d2r)); // west
    CHECK(Near(gmsky::GmodYawToSkyrimHeading(-90.0), 180.0 * d2r)); // south
    CHECK(Near(gmsky::GmodPitchToSkyrimPitch(45.0), 45.0 * d2r));
    // Step 3b: heading back to Source yaw, the exact inverse.
    CHECK(Near(gmsky::SkyrimHeadingToGmodYaw(0.0), 90.0));             // north
    CHECK(Near(gmsky::SkyrimHeadingToGmodYaw(90.0 * d2r), 0.0));       // east
    CHECK(Near(gmsky::SkyrimHeadingToGmodYaw(270.0 * d2r), -180.0));   // west
    CHECK(Near(gmsky::SkyrimHeadingToGmodYaw(180.0 * d2r), -90.0));    // south
    for (double y = -179.0; y < 180.0; y += 7.0) {
        CHECK(Near(gmsky::SkyrimHeadingToGmodYaw(gmsky::GmodYawToSkyrimHeading(y)), y));
    }
}

// The three slot numbers (middle, writer, reader) are always 0, 1, 2 in some order.
static bool Permutation(uint32_t s) {
    const uint32_t m = s & 3u, w = (s >> GMSKY_OVS_WRITER_SHIFT) & 3u, r = (s >> GMSKY_OVS_READER_SHIFT) & 3u;
    return m < 3 && w < 3 && r < 3 && m != w && m != r && w != r;
}

static void TestOverlayStates() {
    uint32_t w = 9, r = 9, got = 9, next = 0;
    // Zero-filled block; writer first, then reader.
    uint32_t s = gmsky::ov::Attach(0, true, &w);
    s = gmsky::ov::Attach(s, false, &r);
    CHECK(Permutation(s) && w == 1 && r == 2);
    CHECK(!gmsky::ov::Take(s, &next, &got));  // nothing published yet
    s = gmsky::ov::Publish(s, &w);            // writer's slot 1 becomes the middle, writer gets 0
    CHECK(Permutation(s) && (s & 3u) == 1 && w == 0 && (s & GMSKY_OVS_NEW));
    CHECK(gmsky::ov::Take(s, &next, &got) && got == 1);
    s = next;
    CHECK(Permutation(s) && !(s & GMSKY_OVS_NEW) && !gmsky::ov::Take(s, &next, &got));
    // Reader first on a fresh block, then the writer: different slots.
    s = gmsky::ov::Attach(0, false, &r);
    s = gmsky::ov::Attach(s, true, &w);
    CHECK(Permutation(s) && r == 2 && w == 1);
    // Skyrim restarts: detaches (or crashes, bit still set), re-attaches: same slot, still a permutation.
    for (int i = 0; i < 5; ++i) s = gmsky::ov::Publish(s, &w);
    const uint32_t before = (s >> GMSKY_OVS_READER_SHIFT) & 3u;
    s = gmsky::ov::Attach(s, false, &r);  // crashed: bit still set
    CHECK(Permutation(s) && r == before);
    s = gmsky::ov::Attach(gmsky::ov::Detach(s, false), false, &r);  // detached properly
    CHECK(Permutation(s) && r == before);
    // Garry's Mod restarts while Skyrim holds a slot.
    s = gmsky::ov::Attach(gmsky::ov::Detach(s, true), true, &w);
    CHECK(Permutation(s));
}

int main() {
    TestSlot();
    TestPosition();
    TestAngles();
    TestOverlayStates();
    std::printf("sizeof(GmskyShared)=%zu  player_state@%zu  failures=%d\n", sizeof(GmskyShared),
                offsetof(GmskyShared, player_state), g_failures);
    std::printf(g_failures ? "PROTOCOL TEST FAILED\n" : "PROTOCOL TEST PASSED\n");
    return g_failures ? 1 : 0;
}
