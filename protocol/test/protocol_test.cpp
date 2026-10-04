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
    // 72 Garry's Mod units = 128 Skyrim units (one player height) with the default scale.
    const double g[3] = {72.0, 0.0, -12000.0 + 72.0};
    s = gmsky::GmodToSkyrim(g, anchor, origin);
    CHECK(Near(s.x, origin[0] + 128.0) && Near(s.z, origin[2] + 128.0));
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
}

int main() {
    TestSlot();
    TestPosition();
    TestAngles();
    std::printf("sizeof(GmskyShared)=%zu  player_state@%zu  failures=%d\n", sizeof(GmskyShared),
                offsetof(GmskyShared, player_state), g_failures);
    std::printf(g_failures ? "PROTOCOL TEST FAILED\n" : "PROTOCOL TEST PASSED\n");
    return g_failures ? 1 : 0;
}
