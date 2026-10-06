// Tests for gmsky_protocol.h. Build and run: see protocol/CMakeLists.txt.
// The layout itself is checked at compile time by the static_asserts in the header.
#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

#include "gmsky_protocol.h"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

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

// Step G2: the C++ render ring reader (what Skyrim runs) against the C++ copy of the writer;
// the same cases as protocol/src/render.rs's tests.
static void TestRenderRing() {
    std::vector<uint64_t> block(GMSKY_RND_MAP_SIZE / 8, 0);
    auto* base = reinterpret_cast<uint8_t*>(block.data());

    // A model: header, one part, 4 vertices on 2 bones, 2 triangles.
    std::vector<uint8_t> model(sizeof(GmskyRndModel) + sizeof(GmskyRndPart) + 4 * sizeof(GmskyRndVertex) + 6 * 4);
    GmskyRndModel mh{5, 4, 6, 1, 2, 0};
    GmskyRndPart part{0, 6, 7, GMSKY_RND_MAT_ALPHATEST, 0.5f, 0, 0.0f, 0.0f};
    std::memcpy(model.data(), &mh, sizeof(mh));
    std::memcpy(model.data() + sizeof(mh), &part, sizeof(part));
    for (int k = 0; k < 4; ++k) {
        GmskyRndVertex v{{float(k), 1, 2}, {0, 0, 1}, {1, 0, 0, 1}, 0.25f, 0.75f, {uint8_t(k & 1), 0, 0, 0}, {255, 0, 0, 0}};
        std::memcpy(model.data() + sizeof(mh) + sizeof(part) + k * sizeof(v), &v, sizeof(v));
    }
    const uint32_t idx[6] = {0, 1, 2, 0, 2, 3};
    std::memcpy(model.data() + model.size() - sizeof(idx), idx, sizeof(idx));
    CHECK(gmsky::rnd::Write(base, GMSKY_RND_MODEL, model.data(), uint32_t(model.size())));

    // A frame: one object with 2 matrices.
    std::vector<uint8_t> frame(sizeof(GmskyRndFrame) + sizeof(GmskyRndObject) + 2 * sizeof(GmskyRndMatrix));
    GmskyRndFrame fh{99, -4, 3, 1, 2, 0};
    GmskyRndObject obj{5, 0, 2, 0xFF00FF80u, 0, 0};
    GmskyRndMatrix mat{};
    mat.m[0][3] = 12.5f;
    std::memcpy(frame.data(), &fh, sizeof(fh));
    std::memcpy(frame.data() + sizeof(fh), &obj, sizeof(obj));
    std::memcpy(frame.data() + sizeof(fh) + sizeof(obj), &mat, sizeof(mat));
    CHECK(gmsky::rnd::Write(base, GMSKY_RND_FRAME, frame.data(), uint32_t(frame.size())));

    // An 8x4 BC1 texture (2 blocks).
    std::vector<uint8_t> tex(sizeof(GmskyRndTexture) + 16, 0xAB);
    GmskyRndTexture th{3, 8, 4, GMSKY_RND_TEX_BC1, 16, 0};
    std::memcpy(tex.data(), &th, sizeof(th));
    CHECK(gmsky::rnd::Write(base, GMSKY_RND_TEXTURE, tex.data(), uint32_t(tex.size())));

    int seen = 0;
    bool bad = false;
    gmsky::rnd::Drain(base, [&](uint32_t type, const uint8_t* p, uint32_t bytes) {
        if (type == GMSKY_RND_MODEL) {
            gmsky::rnd::ModelView mv{};
            CHECK(gmsky::rnd::ParseModel(p, bytes, mv) == nullptr);
            CHECK(mv.model.id == 5 && mv.parts[0].texture_id == 7 && mv.vertices[2].pos[0] == 2.0f && mv.indices[5] == 3);
            // Broken copies are caught.
            std::vector<uint8_t> broken(p, p + bytes);
            const uint32_t pastEnd = 9;
            std::memcpy(broken.data() + bytes - 4, &pastEnd, 4);
            CHECK(gmsky::rnd::ParseModel(broken.data(), bytes, mv) != nullptr);
            CHECK(gmsky::rnd::ParseModel(p, bytes - 4, mv) != nullptr);
            ++seen;
        } else if (type == GMSKY_RND_FRAME) {
            gmsky::rnd::FrameView fv{};
            CHECK(gmsky::rnd::ParseFrame(p, bytes, fv) == nullptr);
            CHECK(fv.frame.frame == 99 && fv.frame.origin_epoch == 3 && fv.objects[0].matrix_count == 2 && fv.matrices[0].m[0][3] == 12.5f);
            CHECK(gmsky::rnd::ParseFrame(p, bytes - sizeof(GmskyRndMatrix), fv) != nullptr);
            ++seen;
        } else if (type == GMSKY_RND_TEXTURE) {
            GmskyRndTexture t{};
            const uint8_t* px = nullptr;
            CHECK(gmsky::rnd::ParseTexture(p, bytes, t, px) == nullptr);
            CHECK(t.id == 3 && px[15] == 0xAB);
            CHECK(gmsky::rnd::ParseTexture(p, bytes - 1, t, px) != nullptr);
            ++seen;
        }
    }, ~0ull, bad);
    CHECK(seen == 3 && !bad);
    CHECK(gmsky::rnd::TextureBytes(5, 5, GMSKY_RND_TEX_BC3) == 64 && gmsky::rnd::TextureBytes(5, 5, 99) == 0);

    // Wrap-around: 3 MB messages in order and whole; a full ring refuses.
    std::vector<uint8_t> big(3000011);
    uint32_t nextW = 0, nextR = 0;
    int refused = 0;
    for (int round = 0; round < 6; ++round) {
        for (;;) {
            std::memset(big.data(), int(nextW % 251), big.size());
            std::memcpy(big.data(), &nextW, 4);
            if (!gmsky::rnd::Write(base, GMSKY_RND_TEXTURE, big.data(), uint32_t(big.size()))) {
                ++refused;
                break;
            }
            ++nextW;
        }
        gmsky::rnd::Drain(base, [&](uint32_t, const uint8_t* p, uint32_t bytes) {
            uint32_t n = 0;
            std::memcpy(&n, p, 4);
            CHECK(bytes == 3000011 && n == nextR && p[bytes - 1] == uint8_t(n % 251));
            ++nextR;
        }, round % 2 ? ~0ull : 20000000ull, bad);
        CHECK(!bad);
    }
    gmsky::rnd::Drain(base, [&](uint32_t, const uint8_t*, uint32_t) { ++nextR; }, ~0ull, bad);
    CHECK(nextR == nextW && refused == 6 && nextW > 60);

    // The fill rule's case: room for C, but not for C plus the skipped end: refused, B intact.
    std::fill(block.begin(), block.end(), 0);
    std::vector<uint8_t> a(30u << 20, 1), b((30u << 20) - 8, 2), c((31u << 20) - 8, 3);
    CHECK(gmsky::rnd::Write(base, GMSKY_RND_TEXTURE, a.data(), uint32_t(a.size() - 8)));
    CHECK(gmsky::rnd::Write(base, GMSKY_RND_TEXTURE, b.data(), uint32_t(b.size())));
    gmsky::rnd::Drain(base, [&](uint32_t, const uint8_t*, uint32_t) {}, 1, bad);  // only A
    CHECK(!gmsky::rnd::Write(base, GMSKY_RND_TEXTURE, c.data(), uint32_t(c.size())));
    bool bIntact = false;
    gmsky::rnd::Drain(base, [&](uint32_t, const uint8_t* p, uint32_t bytes) { bIntact = bytes == b.size() && p[0] == 2 && p[bytes - 1] == 2; }, ~0ull, bad);
    CHECK(bIntact);

    // Garbage in the ring: flagged, everything pending skipped.
    std::fill(block.begin(), block.end(), 0);
    const GmskyColMsgHeader junk{GMSKY_RND_FRAME, 0x7FFFFFF0u};
    std::memcpy(base + GMSKY_RND_DATA_OFF, &junk, sizeof(junk));
    gmsky::rnd::Counter(base, GMSKY_RND_HEAD_OFF).store(64);
    int calls = 0;
    gmsky::rnd::Drain(base, [&](uint32_t, const uint8_t*, uint32_t) { ++calls; }, ~0ull, bad);
    CHECK(bad && calls == 0 && gmsky::rnd::Counter(base, GMSKY_RND_TAIL_OFF).load() == 64);
}

// `protocol_test --read-live N`: reads the real render block for N seconds with the same C++
// reader Skyrim uses (attaches like Skyrim: skips the unread stream, adds 1 to reader_epoch), and
// checks every message. With tools/fakes fake_render writing, this tests the Rust writer against
// the C++ reader through real shared memory, without either game.
static int ReadLive(int a_seconds) {
    HANDLE mapping = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, 0, GMSKY_RND_MAP_SIZE, GMSKY_RND_SHM_NAME);
    auto* base = mapping ? static_cast<uint8_t*>(MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, GMSKY_RND_MAP_SIZE)) : nullptr;
    if (!base) {
        std::printf("READ LIVE FAILED: can't map the render block\n");
        return 1;
    }
    auto& ctl = *reinterpret_cast<GmskyRenderCtl*>(base);
    const auto start = GetTickCount64();
    while (std::atomic_ref<uint32_t>(ctl.magic).load() != GMSKY_RND_MAGIC && GetTickCount64() - start < 3000) Sleep(10);
    if (ctl.magic != GMSKY_RND_MAGIC || ctl.version != GMSKY_RND_VERSION) {
        std::printf("READ LIVE FAILED: block not initialized by a writer (magic %08x version %u)\n", ctl.magic, ctl.version);
        return 1;
    }
    gmsky::rnd::Counter(base, GMSKY_RND_TAIL_OFF).store(gmsky::rnd::Counter(base, GMSKY_RND_HEAD_OFF).load());
    const uint32_t epoch = std::atomic_ref<uint32_t>(ctl.reader_epoch).fetch_add(1) + 1;
    uint64_t counts[6]{}, badMessages = 0, badRing = 0, framesWithModel = 0, lastFrame = 0, framesOutOfOrder = 0;
    bool haveModel = false, haveTexture = false, resetForUs = false;
    float lastX = 0;
    const auto end = GetTickCount64() + uint64_t(a_seconds) * 1000;
    while (GetTickCount64() < end) {
        bool bad = false;
        gmsky::rnd::Drain(base, [&](uint32_t type, const uint8_t* p, uint32_t bytes) {
            ++counts[type < 6 ? type : 0];
            if (type == GMSKY_RND_RESET) {
                GmskyRndReset r{};
                std::memcpy(&r, p, sizeof(r));
                resetForUs |= r.reader_epoch == epoch;
                haveModel = haveTexture = false;
            } else if (type == GMSKY_RND_MODEL) {
                gmsky::rnd::ModelView mv{};
                const char* err = gmsky::rnd::ParseModel(p, bytes, mv);
                badMessages += err ? 1 : 0;
                haveModel |= !err;
                if (!err && counts[GMSKY_RND_MODEL] == 1) std::printf("  model %u: %u vertices, %u triangles, %u parts, %u bones\n", mv.model.id, mv.model.vertex_count, mv.model.index_count / 3, mv.model.part_count, mv.model.bone_count);
            } else if (type == GMSKY_RND_TEXTURE) {
                GmskyRndTexture t{};
                const uint8_t* px = nullptr;
                const char* err = gmsky::rnd::ParseTexture(p, bytes, t, px);
                badMessages += err ? 1 : 0;
                haveTexture |= !err;
                if (!err && counts[GMSKY_RND_TEXTURE] == 1) std::printf("  texture %u: %ux%u format %u, first pixel %02X %02X %02X %02X\n", t.id, t.width, t.height, t.format, px[0], px[1], px[2], px[3]);
            } else if (type == GMSKY_RND_FRAME) {
                gmsky::rnd::FrameView fv{};
                const char* err = gmsky::rnd::ParseFrame(p, bytes, fv);
                badMessages += err ? 1 : 0;
                if (!err) {
                    framesOutOfOrder += lastFrame && fv.frame.frame <= lastFrame ? 1 : 0;
                    lastFrame = fv.frame.frame;
                    framesWithModel += haveModel && haveTexture ? 1 : 0;
                    if (fv.frame.matrix_count) lastX = fv.matrices[0].m[0][3];
                }
            }
        }, ~0ull, bad);
        badRing += bad ? 1 : 0;
        Sleep(5);
    }
    std::printf("read live: reader epoch %u; %llu reset (%s), %llu texture, %llu model, %llu forget, %llu frame, %llu unknown; %llu bad messages, %llu bad ring, %llu frames out of order; %llu frames after the model; last frame %llu at x %.1f\n",
        epoch, counts[1], resetForUs ? "one answered us" : "none for us", counts[2], counts[3], counts[4], counts[5], counts[0], badMessages, badRing, framesOutOfOrder, framesWithModel, lastFrame, lastX);
    const bool ok = resetForUs && haveModel && haveTexture && framesWithModel > uint64_t(a_seconds) * 30 && !badMessages && !badRing && !counts[0] && !framesOutOfOrder;
    std::printf(ok ? "READ LIVE PASSED\n" : "READ LIVE FAILED\n");
    UnmapViewOfFile(base);
    CloseHandle(mapping);
    return ok ? 0 : 1;
}

int main(int argc, char** argv) {
    if (argc >= 2 && std::strcmp(argv[1], "--read-live") == 0) {
        return ReadLive(argc >= 3 ? std::atoi(argv[2]) : 5);
    }
    TestSlot();
    TestPosition();
    TestAngles();
    TestOverlayStates();
    TestRenderRing();
    std::printf("sizeof(GmskyShared)=%zu  player_state@%zu  failures=%d\n", sizeof(GmskyShared),
                offsetof(GmskyShared, player_state), g_failures);
    std::printf(g_failures ? "PROTOCOL TEST FAILED\n" : "PROTOCOL TEST PASSED\n");
    return g_failures ? 1 : 0;
}
