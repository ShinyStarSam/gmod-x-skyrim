// Geometry route, step G1 (DESIGN.md §12, "Garry's Mod's things drawn inside Skyrim"): Skyrim
// draws things itself, inside its own frame. Adapted from SkyCraft (MIT), skse/src/WorldRender.cpp:
// a hook right after the call to Main::RenderWorld (REL::ID(36559) + 0x85E, checked to be a call to
// REL::ID(107142) before patching). There Skyrim's 3D scene is complete and its post-processing
// (tone mapping, bloom, TAA) hasn't run, so what we draw into its HDR scene target with its main
// depth buffer bound is depth-tested against Skyrim's world, writes Skyrim's depth, and goes
// through Skyrim's post-processing like its own objects. Motion vectors are written too, so
// Skyrim's TAA doesn't smear it. Camera: Skyrim's own world -> clip matrix of this frame, re-based
// on the camera position (SkyCraft's RenderPasses); every graphics setting is saved and put back
// (SkyCraft's StateBackup).
//
// G1 draws only a fixed test cube (70 units = 1 m, white, a fixed light), placed once 300 units in
// front of the camera on the first frame after a save loads. Garry's Mod's objects come in G2-G4.
#include "World.h"

#include "Draw.h"
#include "Input.h"
#include "Render.h"

#include <d3d11.h>
#include <d3dcompiler.h>

#include <mutex>

namespace gmsky::World
{
	namespace
	{
		// G1's test cube; off once Garry's Mod's objects are drawn (G3).
		constexpr bool kTestCube = true;
		constexpr float kCubeSize = 70.0f;      // Skyrim units (SkyCraft: 70 per block)
		constexpr float kCubeDistance = 300.0f;  // in front of the camera, level
		constexpr float kCubeDrop = 60.0f;       // centre this far below the camera

		template <class T>
		void Release(T*& a_ptr)
		{
			if (a_ptr) {
				a_ptr->Release();
				a_ptr = nullptr;
			}
		}

		int64_t Qpc()
		{
			LARGE_INTEGER q{};
			QueryPerformanceCounter(&q);
			return q.QuadPart;
		}

		double QpcMs(int64_t a_ticks)
		{
			static const double freq = [] { LARGE_INTEGER f{}; QueryPerformanceFrequency(&f); return double(f.QuadPart); }();
			return double(a_ticks) * 1000.0 / freq;
		}

		constexpr char kShader[] = R"(
cbuffer Frame : register(b0)
{
	row_major float4x4 viewProj;      // camera-relative Skyrim units -> clip (this frame)
	row_major float4x4 prevViewProj;  // last frame's camera, re-based on this frame's (motion vectors)
	float4 offset;                    // the object's centre, camera-relative
	float4 lightDir;                  // towards the light (Skyrim axes)
};

struct VSIn
{
	float3 pos : POSITION;
	float3 normal : NORMAL;
};
struct VSOut
{
	float4 pos : SV_Position;
	float3 normal : NORMAL;
	float4 curClip : TEXCOORD0;
	float4 prevClip : TEXCOORD1;
};
struct PSOut
{
	float4 color : SV_Target0;
	float4 motion : SV_Target1;  // Skyrim's motion vectors (for its TAA), when bound
};

VSOut VSMain(VSIn i)
{
	VSOut  o;
	float3 rel = offset.xyz + i.pos;
	o.pos = mul(viewProj, float4(rel, 1.0));
	o.normal = i.normal;
	o.curClip = o.pos;
	o.prevClip = mul(prevViewProj, float4(rel, 1.0));  // the cube doesn't move
	return o;
}

PSOut PSMain(VSOut i)
{
	PSOut  po;
	float2 cur = i.curClip.xy / i.curClip.w, prev = i.prevClip.xy / i.prevClip.w;
	po.motion = float4(float2(-0.5, 0.5) * (cur - prev), 0.0, 1.0);  // Skyrim's convention: previous uv - current uv
	float lit = 0.35 + 0.65 * saturate(dot(normalize(i.normal), lightDir.xyz));
	po.color = float4(lit.xxx, 1.0);
	return po;
}
)";

		struct FrameConstants
		{
			float viewProj[4][4];
			float prevViewProj[4][4];
			float offset[4];
			float lightDir[4];
		};
		static_assert(sizeof(FrameConstants) % 16 == 0);

		struct Vertex
		{
			float pos[3];
			float normal[3];
		};

		ID3D11Device* g_device = nullptr;
		ID3D11VertexShader* g_vs = nullptr;
		ID3D11PixelShader* g_ps = nullptr;
		ID3D11InputLayout* g_layout = nullptr;
		ID3D11Buffer* g_frameCb = nullptr;
		ID3D11Buffer* g_cubeVb = nullptr;
		ID3D11RasterizerState* g_raster = nullptr;
		ID3D11DepthStencilState* g_depthWrite = nullptr;     // standard Z (0 near, 1 far)
		ID3D11DepthStencilState* g_depthWriteRev = nullptr;  // reversed Z
		ID3D11BlendState* g_solidBlend = nullptr;
		bool g_initFailed = false;
		bool g_hooked = false;
		bool g_targetsChecked = false, g_targetsOk = false;

		double g_prevW2C[4][4]{};
		bool g_havePrevW2C = false;

		// The cube's place in Skyrim's world (render thread); placed again after a load.
		std::atomic<bool> g_placeCube{ true };
		double g_cube[3]{};
		bool g_cubePlaced = false;

		// Counters since the last LogSecond (render thread writes, game thread reads).
		std::atomic<uint32_t> g_calls{ 0 }, g_drawn{ 0 }, g_skipMenu{ 0 }, g_skipNoCamera{ 0 }, g_skipTargets{ 0 }, g_skipInit{ 0 };
		std::atomic<int64_t> g_drawTicks{ 0 }, g_drawMaxTicks{ 0 };

		// What the last drawn frame saw, for the once-a-second line.
		struct Seen
		{
			bool valid = false;
			double cam[3]{};
			double cube[3]{};
			bool onScreen = false;
			float screenX = 0, screenY = 0, distance = 0;
			bool reversed = false;
			const char* depthFrom = "";
		};
		std::mutex g_seenLock;
		Seen g_seen;

		bool Compile(const char* a_entry, const char* a_target, ID3DBlob** a_out)
		{
			ID3DBlob* errors = nullptr;
			const auto hr = D3DCompile(kShader, sizeof(kShader) - 1, "gmsky_world", nullptr, nullptr, a_entry, a_target, D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, a_out, &errors);
			if (FAILED(hr)) {
				logger::error("in-frame drawing: shader {} failed: {}", a_entry, errors ? static_cast<const char*>(errors->GetBufferPointer()) : "?");
				Release(errors);
				return false;
			}
			Release(errors);
			return true;
		}

		bool Init(ID3D11Device* a_device)
		{
			if (g_device) {
				return true;
			}
			if (g_initFailed) {
				return false;
			}
			ID3DBlob *vsBlob = nullptr, *psBlob = nullptr;
			if (!Compile("VSMain", "vs_5_0", &vsBlob) || !Compile("PSMain", "ps_5_0", &psBlob)) {
				Release(vsBlob);
				g_initFailed = true;
				return false;
			}
			a_device->CreateVertexShader(vsBlob->GetBufferPointer(), vsBlob->GetBufferSize(), nullptr, &g_vs);
			a_device->CreatePixelShader(psBlob->GetBufferPointer(), psBlob->GetBufferSize(), nullptr, &g_ps);
			const D3D11_INPUT_ELEMENT_DESC elements[] = {
				{ "POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "NORMAL", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 12, D3D11_INPUT_PER_VERTEX_DATA, 0 },
			};
			a_device->CreateInputLayout(elements, 2, vsBlob->GetBufferPointer(), vsBlob->GetBufferSize(), &g_layout);
			Release(vsBlob);
			Release(psBlob);

			D3D11_BUFFER_DESC cbd{};
			cbd.ByteWidth = sizeof(FrameConstants);
			cbd.Usage = D3D11_USAGE_DYNAMIC;
			cbd.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
			cbd.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
			a_device->CreateBuffer(&cbd, nullptr, &g_frameCb);

			// The cube: 6 faces, 2 triangles each, centred on 0.
			std::vector<Vertex> verts;
			const float h = kCubeSize * 0.5f;
			for (int axis = 0; axis < 3; ++axis) {
				for (int sign = -1; sign <= 1; sign += 2) {
					float n[3]{};
					n[axis] = float(sign);
					const int u = (axis + 1) % 3, v = (axis + 2) % 3;
					float corner[4][3]{};
					const float us[4] = { -h, h, h, -h }, vs[4] = { -h, -h, h, h };
					for (int k = 0; k < 4; ++k) {
						corner[k][axis] = h * float(sign);
						corner[k][u] = us[k];
						corner[k][v] = vs[k];
					}
					for (int k : { 0, 1, 2, 0, 2, 3 }) {
						verts.push_back({ { corner[k][0], corner[k][1], corner[k][2] }, { n[0], n[1], n[2] } });
					}
				}
			}
			D3D11_BUFFER_DESC vbd{};
			vbd.ByteWidth = UINT(verts.size() * sizeof(Vertex));
			vbd.Usage = D3D11_USAGE_IMMUTABLE;
			vbd.BindFlags = D3D11_BIND_VERTEX_BUFFER;
			D3D11_SUBRESOURCE_DATA init{ verts.data(), 0, 0 };
			a_device->CreateBuffer(&vbd, &init, &g_cubeVb);

			D3D11_RASTERIZER_DESC rd{};
			rd.FillMode = D3D11_FILL_SOLID;
			rd.CullMode = D3D11_CULL_NONE;  // the cube's winding doesn't matter for G1
			rd.DepthClipEnable = TRUE;
			a_device->CreateRasterizerState(&rd, &g_raster);

			D3D11_DEPTH_STENCIL_DESC dd{};
			dd.DepthEnable = TRUE;
			dd.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
			dd.DepthFunc = D3D11_COMPARISON_LESS_EQUAL;
			a_device->CreateDepthStencilState(&dd, &g_depthWrite);
			dd.DepthFunc = D3D11_COMPARISON_GREATER_EQUAL;
			a_device->CreateDepthStencilState(&dd, &g_depthWriteRev);

			// Colour RGB (Skyrim's alpha left alone), every channel of the motion vectors.
			D3D11_BLEND_DESC sb{};
			sb.IndependentBlendEnable = TRUE;
			sb.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_RED | D3D11_COLOR_WRITE_ENABLE_GREEN | D3D11_COLOR_WRITE_ENABLE_BLUE;
			for (int k = 1; k < 8; ++k) {
				sb.RenderTarget[k].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
			}
			a_device->CreateBlendState(&sb, &g_solidBlend);

			const bool ok = g_vs && g_ps && g_layout && g_frameCb && g_cubeVb && g_raster && g_depthWrite && g_depthWriteRev && g_solidBlend;
			if (!ok) {
				logger::error("in-frame drawing: failed to create its graphics objects");
				g_initFailed = true;
				return false;
			}
			g_device = a_device;
			g_device->AddRef();
			logger::info("in-frame drawing: ready ({} cube vertices)", verts.size());
			return true;
		}

		// Every graphics setting we touch, saved and put back (SkyCraft's StateBackup). Binding a
		// render target or depth buffer unbinds it wherever it's bound for reading, so every
		// stage's shader resources go back too.
		struct StateBackup
		{
			static constexpr UINT kSlots = D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT;
			ID3D11RenderTargetView* rtv[D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT]{};
			ID3D11DepthStencilView* dsv{};
			ID3D11BlendState* blend{};
			float factor[4]{};
			UINT mask{};
			ID3D11RasterizerState* raster{};
			ID3D11DepthStencilState* depth{};
			UINT stencil{};
			D3D11_VIEWPORT vps[D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE]{};
			UINT vpCount{ D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE };
			D3D11_PRIMITIVE_TOPOLOGY topo{};
			ID3D11InputLayout* layout{};
			ID3D11Buffer* vb{};
			UINT stride{}, offset{};
			ID3D11VertexShader* vs{};
			ID3D11PixelShader* ps{};
			ID3D11Buffer* vsCb{};
			ID3D11Buffer* psCb{};
			ID3D11ShaderResourceView* vsAll[kSlots]{};
			ID3D11ShaderResourceView* psAll[kSlots]{};
			ID3D11ShaderResourceView* csAll[kSlots]{};

			void Save(ID3D11DeviceContext* a_c)
			{
				a_c->OMGetRenderTargets(D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT, rtv, &dsv);
				a_c->OMGetBlendState(&blend, factor, &mask);
				a_c->RSGetState(&raster);
				a_c->OMGetDepthStencilState(&depth, &stencil);
				a_c->RSGetViewports(&vpCount, vps);
				a_c->IAGetPrimitiveTopology(&topo);
				a_c->IAGetInputLayout(&layout);
				a_c->IAGetVertexBuffers(0, 1, &vb, &stride, &offset);
				a_c->VSGetShader(&vs, nullptr, nullptr);
				a_c->PSGetShader(&ps, nullptr, nullptr);
				a_c->VSGetConstantBuffers(0, 1, &vsCb);
				a_c->PSGetConstantBuffers(0, 1, &psCb);
				a_c->VSGetShaderResources(0, kSlots, vsAll);
				a_c->PSGetShaderResources(0, kSlots, psAll);
				a_c->CSGetShaderResources(0, kSlots, csAll);
			}

			void Restore(ID3D11DeviceContext* a_c)
			{
				a_c->OMSetRenderTargets(D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT, rtv, dsv);
				a_c->OMSetBlendState(blend, factor, mask);
				a_c->RSSetState(raster);
				a_c->OMSetDepthStencilState(depth, stencil);
				a_c->RSSetViewports(vpCount, vps);
				a_c->IASetPrimitiveTopology(topo);
				a_c->IASetInputLayout(layout);
				a_c->IASetVertexBuffers(0, 1, &vb, &stride, &offset);
				a_c->VSSetShader(vs, nullptr, 0);
				a_c->PSSetShader(ps, nullptr, 0);
				a_c->VSSetConstantBuffers(0, 1, &vsCb);
				a_c->PSSetConstantBuffers(0, 1, &psCb);
				a_c->VSSetShaderResources(0, kSlots, vsAll);
				a_c->PSSetShaderResources(0, kSlots, psAll);
				a_c->CSSetShaderResources(0, kSlots, csAll);
				for (auto* list : { vsAll, psAll, csAll }) {
					for (UINT k = 0; k < kSlots; ++k) {
						if (list[k]) {
							list[k]->Release();
						}
					}
				}
				for (auto*& r : rtv) {
					Release(r);
				}
				Release(dsv);
				Release(blend);
				Release(raster);
				Release(depth);
				Release(layout);
				Release(vb);
				Release(vs);
				Release(ps);
				Release(vsCb);
				Release(psCb);
			}
		};

		// Once: what Skyrim has here (SkyCraft's check). Drawing needs an HDR scene target and a
		// depth buffer of the same size, without multisampling.
		bool CheckTargets(ID3D11DeviceContext* a_context, RE::BSGraphics::RendererData& a_rd, ID3D11Texture2D* a_sceneTex, ID3D11Texture2D* a_depthTex)
		{
			if (g_targetsChecked) {
				return g_targetsOk;
			}
			g_targetsChecked = true;
			D3D11_TEXTURE2D_DESC sd{}, dd{}, md{};
			a_sceneTex->GetDesc(&sd);
			a_depthTex->GetDesc(&dd);
			const auto& motion = a_rd.renderTargets[RE::RENDER_TARGETS::kMOTION_VECTOR];
			if (motion.texture) {
				reinterpret_cast<ID3D11Texture2D*>(motion.texture)->GetDesc(&md);
			}
			ID3D11RenderTargetView* bound[6]{};
			ID3D11DepthStencilView* boundDsv = nullptr;
			a_context->OMGetRenderTargets(6, bound, &boundDsv);
			std::string slots;
			for (auto*& b : bound) {
				int index = b ? -2 : -1;
				for (int i = 0; b && i < RE::RENDER_TARGETS::kTOTAL; ++i) {
					if (reinterpret_cast<ID3D11RenderTargetView*>(a_rd.renderTargets[i].RTV) == b) {
						index = i;
					}
				}
				slots += std::format("{} ", index);
				Release(b);
			}
			Release(boundDsv);
			const bool hdr = sd.Format == DXGI_FORMAT_R16G16B16A16_FLOAT || sd.Format == DXGI_FORMAT_R11G11B10_FLOAT || sd.Format == DXGI_FORMAT_R32G32B32A32_FLOAT;
			const bool sizes = sd.Width == dd.Width && sd.Height == dd.Height && sd.SampleDesc.Count == 1 && dd.SampleDesc.Count == 1;
			g_targetsOk = hdr && sizes;
			logger::info("in-frame drawing: scene {}x{} format {} ({}), depth {}x{} format {}, motion vectors {}x{} format {}; bound here: targets [ {}] depth {}; {}",
				sd.Width, sd.Height, int(sd.Format), hdr ? "HDR" : "not HDR", dd.Width, dd.Height, int(dd.Format), md.Width, md.Height, int(md.Format), slots,
				boundDsv ? "yes" : "none", g_targetsOk ? "drawing here" : "UNEXPECTED, nothing drawn");
			return g_targetsOk;
		}

		void DrawInFrame()
		{
			g_calls.fetch_add(1, std::memory_order_relaxed);
			// G2: read what Garry's Mod sent (every frame, menus or not, so the ring never fills).
			Render::Drain();
			if (!kTestCube) {
				return;
			}
			auto* camera = RE::Main::WorldRootCamera();
			auto* renderer = RE::BSGraphics::Renderer::GetSingleton();
			if (!camera || !renderer || !RE::PlayerCharacter::GetSingleton() || !RE::PlayerCharacter::GetSingleton()->Get3D()) {
				g_skipNoCamera.fetch_add(1, std::memory_order_relaxed);
				return;
			}
			if (Input::SkyrimMenuOpen()) {
				g_skipMenu.fetch_add(1, std::memory_order_relaxed);
				return;
			}
			auto& rd = renderer->GetRuntimeData();
			auto* context = reinterpret_cast<ID3D11DeviceContext*>(rd.context);
			if (!context) {
				g_skipNoCamera.fetch_add(1, std::memory_order_relaxed);
				return;
			}
			if (!g_device) {
				ID3D11Device* device = nullptr;
				context->GetDevice(&device);
				const bool ok = device && Init(device);
				Release(device);
				if (!ok) {
					g_skipInit.fetch_add(1, std::memory_order_relaxed);
					return;
				}
			}
			const auto& scene = rd.renderTargets[RE::RENDER_TARGETS::kMAIN];
			const auto& motion = rd.renderTargets[RE::RENDER_TARGETS::kMOTION_VECTOR];
			const auto& depth = renderer->GetDepthStencilData().depthStencils[RE::RENDER_TARGETS_DEPTHSTENCIL::kMAIN];
			auto* sceneTex = reinterpret_cast<ID3D11Texture2D*>(scene.texture);
			auto* depthTex = reinterpret_cast<ID3D11Texture2D*>(depth.texture);
			auto* rtv = reinterpret_cast<ID3D11RenderTargetView*>(scene.RTV);
			auto* dsv = reinterpret_cast<ID3D11DepthStencilView*>(depth.views[0]);
			if (!sceneTex || !depthTex || !rtv || !dsv || !CheckTargets(context, rd, sceneTex, depthTex)) {
				g_skipTargets.fetch_add(1, std::memory_order_relaxed);
				return;
			}
			const auto start = Qpc();

			const auto cam = camera->world.translate;
			const auto& R = camera->world.rotate;  // columns: forward, up, right
			if (g_placeCube.exchange(false) || !g_cubePlaced) {
				double fx = R.entry[0][0], fy = R.entry[1][0];
				const double len = std::sqrt(fx * fx + fy * fy);
				if (len > 1e-3) {
					fx /= len, fy /= len;
				} else {
					fx = 1.0, fy = 0.0;
				}
				g_cube[0] = double(cam.x) + fx * kCubeDistance;
				g_cube[1] = double(cam.y) + fy * kCubeDistance;
				g_cube[2] = double(cam.z) - kCubeDrop;
				g_cubePlaced = true;
				logger::info("in-frame drawing: test cube placed at ({:.1f}, {:.1f}, {:.1f}), camera at ({:.1f}, {:.1f}, {:.1f}), size {} units",
					g_cube[0], g_cube[1], g_cube[2], cam.x, cam.y, cam.z, kCubeSize);
			}

			// Camera: Skyrim's own world -> clip matrix, re-based on the camera position so the GPU
			// only sees small camera-relative coordinates (SkyCraft's RenderPasses).
			const auto& w2c = camera->GetRuntimeData().worldToCam;
			FrameConstants fc{};
			for (int r = 0; r < 4; ++r) {
				for (int c = 0; c < 3; ++c) {
					fc.viewProj[r][c] = w2c[r][c];
					fc.prevViewProj[r][c] = float(g_havePrevW2C ? g_prevW2C[r][c] : w2c[r][c]);
				}
				fc.viewProj[r][3] = float(double(w2c[r][3]) + double(w2c[r][0]) * cam.x + double(w2c[r][1]) * cam.y + double(w2c[r][2]) * cam.z);
				const double* pr = g_havePrevW2C ? g_prevW2C[r] : nullptr;
				fc.prevViewProj[r][3] = pr ? float(pr[3] + pr[0] * cam.x + pr[1] * cam.y + pr[2] * cam.z) : fc.viewProj[r][3];
			}
			for (int r = 0; r < 4; ++r) {
				for (int c = 0; c < 4; ++c) {
					g_prevW2C[r][c] = w2c[r][c];
				}
			}
			g_havePrevW2C = true;
			const double rel[3] = { g_cube[0] - cam.x, g_cube[1] - cam.y, g_cube[2] - cam.z };
			fc.offset[0] = float(rel[0]), fc.offset[1] = float(rel[1]), fc.offset[2] = float(rel[2]);
			const float l[3] = { 0.4f, 0.3f, 0.866f };  // a fixed high light, from the south-east
			fc.lightDir[0] = l[0], fc.lightDir[1] = l[1], fc.lightDir[2] = l[2];

			// Depth convention: the finished-frame probe's (it measures Skyrim's buffer itself), else
			// the matrix's (near maps above far = reversed).
			auto ndcZ = [&](float a_dist) {
				const float p[3] = { R.entry[0][0] * a_dist, R.entry[1][0] * a_dist, R.entry[2][0] * a_dist };
				const float z = fc.viewProj[2][0] * p[0] + fc.viewProj[2][1] * p[1] + fc.viewProj[2][2] * p[2] + fc.viewProj[2][3];
				const float w = fc.viewProj[3][0] * p[0] + fc.viewProj[3][1] * p[1] + fc.viewProj[3][2] * p[2] + fc.viewProj[3][3];
				return std::fabs(w) > 1e-6f ? z / w : 0.0f;
			};
			const bool matrixReversed = ndcZ(100.0f) > ndcZ(10000.0f);
			bool probeReversed = false;
			const bool probeKnown = Draw::DepthConvention(probeReversed);
			const bool reversed = probeKnown ? probeReversed : matrixReversed;
			static int lastConvention = -1;
			const int convention = (probeKnown ? 2 : 0) + (reversed ? 1 : 0);
			if (convention != lastConvention) {
				lastConvention = convention;
				logger::info("in-frame drawing: depth {} ({}); the camera's matrix says {}", reversed ? "reversed Z" : "standard Z",
					probeKnown ? "from the depth probe" : "probe not ready, from the matrix", matrixReversed ? "reversed" : "standard");
			}

			static bool checkedMatrix = false;
			if (!checkedMatrix) {
				// SkyCraft's one-time check of the matrix against Skyrim's own projection.
				checkedMatrix = true;
				RE::NiPoint3 fwd{ R.entry[0][0], R.entry[1][0], R.entry[2][0] };
				const auto pt = cam + fwd * 500.0f;
				float sx = 0, sy = 0, sz = 0;
				RE::NiCamera::WorldPtToScreenPt3(w2c, camera->GetRuntimeData2().port, pt, sx, sy, sz, 1e-5f);
				const RE::NiPoint3 d = pt - cam;
				float clip[4];
				for (int r = 0; r < 4; ++r) {
					clip[r] = fc.viewProj[r][0] * d.x + fc.viewProj[r][1] * d.y + fc.viewProj[r][2] * d.z + fc.viewProj[r][3];
				}
				const auto& frustum = camera->GetRuntimeData2().viewFrustum;
				logger::info("in-frame drawing: matrix check: Skyrim screen ({:.3f}, {:.3f}, {:.4f}), ours ({:.3f}, {:.3f}, {:.4f}, w {:.1f}) for 500 units ahead; near {} far {}",
					sx, sy, sz, (clip[0] / clip[3] + 1) * 0.5f, (clip[1] / clip[3] + 1) * 0.5f, clip[2] / clip[3], clip[3], frustum.fNear, frustum.fFar);
			}

			// Where the cube's centre is on screen (for the log).
			{
				float clip[4];
				for (int r = 0; r < 4; ++r) {
					clip[r] = fc.viewProj[r][0] * fc.offset[0] + fc.viewProj[r][1] * fc.offset[1] + fc.viewProj[r][2] * fc.offset[2] + fc.viewProj[r][3];
				}
				std::lock_guard lock(g_seenLock);
				g_seen.valid = true;
				g_seen.cam[0] = cam.x, g_seen.cam[1] = cam.y, g_seen.cam[2] = cam.z;
				std::copy(std::begin(g_cube), std::end(g_cube), g_seen.cube);
				g_seen.distance = float(std::sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]));
				g_seen.onScreen = clip[3] > 1e-3f && std::fabs(clip[0] / clip[3]) <= 1.0f && std::fabs(clip[1] / clip[3]) <= 1.0f;
				g_seen.screenX = clip[3] > 1e-3f ? (clip[0] / clip[3] + 1) * 0.5f : -1.0f;
				g_seen.screenY = clip[3] > 1e-3f ? (1 - clip[1] / clip[3]) * 0.5f : -1.0f;
				g_seen.reversed = reversed;
				g_seen.depthFrom = probeKnown ? "probe" : "matrix";
			}

			StateBackup backup;
			backup.Save(context);
			D3D11_MAPPED_SUBRESOURCE mapped{};
			if (SUCCEEDED(context->Map(g_frameCb, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
				std::memcpy(mapped.pData, &fc, sizeof(fc));
				context->Unmap(g_frameCb, 0);
			}
			D3D11_TEXTURE2D_DESC sd{};
			sceneTex->GetDesc(&sd);
			D3D11_VIEWPORT vp{ 0, 0, float(sd.Width), float(sd.Height), 0, 1 };
			// Unbind the shader resources first: Skyrim may still have its depth bound for reading.
			ID3D11ShaderResourceView* none[StateBackup::kSlots]{};
			context->VSSetShaderResources(0, StateBackup::kSlots, none);
			context->PSSetShaderResources(0, StateBackup::kSlots, none);
			context->CSSetShaderResources(0, StateBackup::kSlots, none);
			auto* motionRtv = motion.RTV && motion.texture ? reinterpret_cast<ID3D11RenderTargetView*>(motion.RTV) : nullptr;
			ID3D11RenderTargetView* targets[2] = { rtv, motionRtv };
			context->OMSetRenderTargets(motionRtv ? 2 : 1, targets, dsv);
			context->RSSetViewports(1, &vp);
			context->RSSetState(g_raster);
			context->OMSetBlendState(g_solidBlend, nullptr, 0xFFFFFFFF);
			context->OMSetDepthStencilState(reversed ? g_depthWriteRev : g_depthWrite, 0);
			context->IASetInputLayout(g_layout);
			context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
			const UINT stride = sizeof(Vertex), zero = 0;
			context->IASetVertexBuffers(0, 1, &g_cubeVb, &stride, &zero);
			context->VSSetShader(g_vs, nullptr, 0);
			context->PSSetShader(g_ps, nullptr, 0);
			context->VSSetConstantBuffers(0, 1, &g_frameCb);
			context->PSSetConstantBuffers(0, 1, &g_frameCb);
			context->Draw(36, 0);
			backup.Restore(context);

			const auto ticks = Qpc() - start;
			g_drawTicks.fetch_add(ticks, std::memory_order_relaxed);
			auto prevMax = g_drawMaxTicks.load(std::memory_order_relaxed);
			while (ticks > prevMax && !g_drawMaxTicks.compare_exchange_weak(prevMax, ticks)) {
			}
			g_drawn.fetch_add(1, std::memory_order_relaxed);
		}

		struct RenderWorldHook
		{
			static void thunk(bool a_unk)
			{
				func(a_unk);
				DrawInFrame();
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};
	}

	void Install()
	{
		if (!REL::Module::IsAE()) {
			logger::warn("in-frame drawing: only set up for Skyrim AE; not hooked");
			return;
		}
		// Main::RenderWorld's one call, in the frame function (SkyCraft's site). Patched only if the
		// bytes there are the call we expect.
		const auto site = REL::ID(36559).address() + 0x85E;
		const auto target = REL::ID(107142).address();
		const auto* code = reinterpret_cast<const std::uint8_t*>(site);
		std::int32_t rel = 0;
		std::memcpy(&rel, code + 1, 4);
		if (code[0] != 0xE8 || site + 5 + static_cast<std::intptr_t>(rel) != target) {
			logger::error("in-frame drawing: Main::RenderWorld's call isn't where expected (byte {:02X}); not hooked", code[0]);
			return;
		}
		RenderWorldHook::func = SKSE::GetTrampoline().write_call<5>(site, RenderWorldHook::thunk);
		g_hooked = true;
		logger::info("in-frame drawing: hooked after Main::RenderWorld (test cube {})", kTestCube ? "on" : "off");
	}

	void OnGameLoaded()
	{
		g_placeCube = true;
	}

	void LogSecond()
	{
		if (!g_hooked) {
			return;
		}
		const auto calls = g_calls.exchange(0), drawn = g_drawn.exchange(0);
		const auto menu = g_skipMenu.exchange(0), noCam = g_skipNoCamera.exchange(0), targets = g_skipTargets.exchange(0), init = g_skipInit.exchange(0);
		const auto ticks = g_drawTicks.exchange(0), maxTicks = g_drawMaxTicks.exchange(0);
		Seen seen;
		{
			std::lock_guard lock(g_seenLock);
			seen = g_seen;
		}
		std::string cube = "not placed";
		if (seen.valid) {
			cube = std::format("cube at ({:.1f}, {:.1f}, {:.1f}), camera ({:.1f}, {:.1f}, {:.1f}), {:.0f} units away, {}", seen.cube[0], seen.cube[1], seen.cube[2],
				seen.cam[0], seen.cam[1], seen.cam[2], seen.distance,
				seen.onScreen ? std::format("on screen at ({:.3f}, {:.3f})", seen.screenX, seen.screenY) : std::string("off screen"));
		}
		logger::info("in-frame drawing: {} calls, {} drawn, skipped: {} menu, {} no camera/player, {} targets, {} init; {:.3f} ms avg, {:.3f} max (CPU); depth {} ({}); {}",
			calls, drawn, menu, noCam, targets, init, drawn ? QpcMs(ticks) / drawn : 0.0, QpcMs(maxTicks), seen.reversed ? "reversed" : "standard", seen.depthFrom, cube);
	}
}
