// Step 2c-1: Garry's Mod's picture (hands, weapon, HUD on a transparent background) drawn over
// Skyrim's finished frame. Adapted from SkyCraft (MIT), skse/src/Overlay.cpp: a hook on entry 8
// (Present) of Skyrim's swap chain function table; in it, the newest picture is taken from the
// overlay block, copied into a texture (Map WRITE_DISCARD), and drawn with one triangle covering
// the screen, premultiplied (out = picture + (1 - alpha) x Skyrim), every setting saved and put
// back. Different from SkyCraft (DESIGN.md "Step 2c in detail"): blended in linear light through
// sRGB formats (Garry's Mod's colours are sRGB-encoded results of linear blending, test 32);
// pictures older than 0.5 s are not drawn.
//
// Steps W3-W4 (Garry's Mod's world layer tested against Skyrim's depth and re-projected onto
// Skyrim's camera) were removed in G8 (DESIGN.md): Skyrim draws Garry's Mod's objects itself
// (World.cpp). Only the hands / HUD picture comes this way now.
#include "Draw.h"

#include "Input.h"
#include "Overlay.h"
#include "Puppet.h"

#include <d3d11.h>
#include <d3dcompiler.h>

namespace gmsky::Draw
{
	namespace
	{
		using PresentFn = HRESULT(WINAPI*)(IDXGISwapChain*, UINT, UINT);
		PresentFn g_originalPresent = nullptr;

		constexpr double kStaleMs = 500.0;

		ID3D11Device* g_device = nullptr;
		ID3D11DeviceContext* g_context = nullptr;
		ID3D11Texture2D* g_texture = nullptr;
		ID3D11ShaderResourceView* g_srv = nullptr;
		UINT g_texW = 0, g_texH = 0;
		bool g_texSrgb = false;
		ID3D11VertexShader* g_vs = nullptr;
		ID3D11PixelShader* g_ps = nullptr;
		ID3D11BlendState* g_blend = nullptr;
		ID3D11SamplerState* g_sampler = nullptr;
		ID3D11RasterizerState* g_raster = nullptr;
		ID3D11DepthStencilState* g_depth = nullptr;
		bool g_initFailed = false;
		bool g_havePicture = false;
		int64_t g_pictureQpc = 0;      // capture time of the picture in the texture
		uint64_t g_pictureFrame = 0;
		uint32_t g_pictureSlot = 0;     // the block slot it came from (ours until the next take)
		// Blending mode, decided on the first draw: 1 = linear (sRGB view), 0 = approximate, -1 = not yet.
		int g_linear = -1;
		std::string g_initError;

		// Counters since the last LogSecond (written by the Present thread, read by the game thread).
		std::atomic<uint64_t> g_presents{ 0 }, g_taken{ 0 }, g_drawn{ 0 };
		std::atomic<uint64_t> g_skipMenu{ 0 }, g_skipPuppet{ 0 }, g_skipNoPicture{ 0 }, g_skipStale{ 0 }, g_skipError{ 0 };
		std::atomic<uint64_t> g_uploadTicks{ 0 }, g_uploadMaxTicks{ 0 }, g_drawTicks{ 0 };
		std::atomic<uint64_t> g_ageSumUs{ 0 }, g_ageMaxUs{ 0 };
		std::atomic<uint32_t> g_presentThread{ 0 };
		std::atomic<uint32_t> g_bbFormat{ 0 }, g_bbW{ 0 }, g_bbH{ 0 };
		std::mutex g_errorMutex;
		std::string g_lastError;

		void Error(std::string a_msg)
		{
			++g_skipError;
			std::lock_guard lock(g_errorMutex);
			g_lastError = std::move(a_msg);
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

		void Max(std::atomic<uint64_t>& a_max, uint64_t a_v)
		{
			uint64_t cur = a_max.load(std::memory_order_relaxed);
			while (a_v > cur && !a_max.compare_exchange_weak(cur, a_v, std::memory_order_relaxed)) {
			}
		}

		template <class T>
		void SafeRelease(T*& a_ptr)
		{
			if (a_ptr) {
				a_ptr->Release();
				a_ptr = nullptr;
			}
		}

		// sRGB formats do the decoding and encoding (DESIGN.md "Step 2c in detail"). Blur behind
		// see-through parts (DESIGN.md "Blur behind the plates", revised): with blur weight
		// w = min(1, a / blurFull), PSMain outputs colour P + (1 - a) w B and alpha
		// 1 - (1 - a)(1 - w), so the premultiplied blend gives P + (1 - a)(w B + (1 - w) S).
		// PSCopy (shrinking, bilinear) and PSBlur (9-tap Gaussian along `dir`) read t1.
		constexpr char kShader[] = R"(
Texture2D overlay : register(t0);
Texture2D source : register(t1);
SamplerState samp : register(s0);
cbuffer Params : register(b0) {
	float blurOn; float blurFull; float2 dir;
};
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut VSMain(uint id : SV_VertexID) {
	VSOut o;
	float2 uv = float2((id << 1) & 2, id & 2);
	o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
	o.uv = uv;
	return o;
}
float4 PSMain(VSOut i) : SV_Target {
	float4 p = overlay.Sample(samp, i.uv);
	if (blurOn < 0.5) return p;
	float w = saturate(p.a / blurFull);
	float3 b = source.Sample(samp, i.uv).rgb;
	return float4(p.rgb + (1 - p.a) * w * b, 1 - (1 - p.a) * (1 - w));
}
float4 PSCopy(VSOut i) : SV_Target { return float4(source.Sample(samp, i.uv).rgb, 1); }
float4 PSBlur(VSOut i) : SV_Target {
	const float wt[5] = { 0.2270270270, 0.1945945946, 0.1216216216, 0.0540540541, 0.0162162162 };
	float3 c = source.Sample(samp, i.uv).rgb * wt[0];
	[unroll] for (int k = 1; k < 5; ++k) {
		c += (source.Sample(samp, i.uv + dir * k).rgb + source.Sample(samp, i.uv - dir * k).rgb) * wt[k];
	}
	return float4(c, 1);
}
)";

		// ---- Blur behind see-through parts ----
		struct alignas(16) Params
		{
			float blurOn;
			float blurFull;
			float dir[2];
		};
		constexpr float kBlurFull = 0.35f;  // HOLOHUD's plates (alpha 94/255 = 0.37) get the full blur
		constexpr int kBlurIterations = 2;  // horizontal + vertical, twice, at quarter size
		ID3D11PixelShader* g_psCopy = nullptr;
		ID3D11PixelShader* g_psBlur = nullptr;
		ID3D11Buffer* g_params = nullptr;
		// Skyrim's frame copied (same format family, read through an sRGB view), then half and two
		// quarter-size float textures.
		ID3D11Texture2D* g_sceneCopy = nullptr;
		ID3D11ShaderResourceView* g_sceneSrv = nullptr;
		struct Target
		{
			ID3D11Texture2D* tex = nullptr;
			ID3D11RenderTargetView* rtv = nullptr;
			ID3D11ShaderResourceView* srv = nullptr;
			UINT w = 0, h = 0;
		};
		Target g_half, g_quarterA, g_quarterB;
		UINT g_blurW = 0, g_blurH = 0;
		DXGI_FORMAT g_blurFormat = DXGI_FORMAT_UNKNOWN;
		bool g_blurBroken = false;
		uint32_t g_pictureFlags = 0;
		std::atomic<uint64_t> g_blurred{ 0 }, g_blurTicks{ 0 };

		bool Compile(const char* a_entry, const char* a_target, ID3DBlob** a_out)
		{
			ID3DBlob* errors = nullptr;
			const auto hr = D3DCompile(kShader, sizeof(kShader) - 1, "gmsky_overlay", nullptr, nullptr, a_entry, a_target,
				D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, a_out, &errors);
			if (FAILED(hr)) {
				g_initError = fmt::format("shader {} failed: {}", a_entry, errors ? static_cast<const char*>(errors->GetBufferPointer()) : "?");
				SafeRelease(errors);
				return false;
			}
			SafeRelease(errors);
			return true;
		}

		bool InitResources(IDXGISwapChain* a_swapChain)
		{
			if (g_device) {
				return true;
			}
			if (g_initFailed) {
				return false;
			}
			if (FAILED(a_swapChain->GetDevice(__uuidof(ID3D11Device), reinterpret_cast<void**>(&g_device)))) {
				g_initFailed = true;
				g_initError = "swap chain GetDevice failed";
				logger::error("overlay draw: {}", g_initError);
				return false;
			}
			g_device->GetImmediateContext(&g_context);
			ID3DBlob *vsBlob = nullptr, *psBlob = nullptr;
			if (!Compile("VSMain", "vs_5_0", &vsBlob) || !Compile("PSMain", "ps_5_0", &psBlob)) {
				SafeRelease(vsBlob);
				g_initFailed = true;
				logger::error("overlay draw: {}", g_initError);
				return false;
			}
			g_device->CreateVertexShader(vsBlob->GetBufferPointer(), vsBlob->GetBufferSize(), nullptr, &g_vs);
			g_device->CreatePixelShader(psBlob->GetBufferPointer(), psBlob->GetBufferSize(), nullptr, &g_ps);
			SafeRelease(vsBlob);
			SafeRelease(psBlob);
			// Blur passes (not fatal if they fail: the overlay is then drawn without blur).
			ID3DBlob *copyBlob = nullptr, *blurBlob = nullptr;
			if (Compile("PSCopy", "ps_5_0", &copyBlob) && Compile("PSBlur", "ps_5_0", &blurBlob)) {
				g_device->CreatePixelShader(copyBlob->GetBufferPointer(), copyBlob->GetBufferSize(), nullptr, &g_psCopy);
				g_device->CreatePixelShader(blurBlob->GetBufferPointer(), blurBlob->GetBufferSize(), nullptr, &g_psBlur);
			} else {
				logger::error("overlay draw: blur shaders: {}", g_initError);
				g_initError.clear();
			}
			SafeRelease(copyBlob);
			SafeRelease(blurBlob);
			D3D11_BLEND_DESC bd{};
			auto& rt = bd.RenderTarget[0];
			rt.BlendEnable = TRUE;
			rt.SrcBlend = D3D11_BLEND_ONE;  // premultiplied
			rt.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
			rt.BlendOp = D3D11_BLEND_OP_ADD;
			rt.SrcBlendAlpha = D3D11_BLEND_ZERO;  // Skyrim's alpha left as it is
			rt.DestBlendAlpha = D3D11_BLEND_ONE;
			rt.BlendOpAlpha = D3D11_BLEND_OP_ADD;
			rt.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
			g_device->CreateBlendState(&bd, &g_blend);

			D3D11_SAMPLER_DESC sd{};
			sd.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;  // exact when sizes match; smooth when scaled
			sd.AddressU = sd.AddressV = sd.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
			sd.MaxLOD = D3D11_FLOAT32_MAX;
			g_device->CreateSamplerState(&sd, &g_sampler);

			D3D11_RASTERIZER_DESC rd{};
			rd.FillMode = D3D11_FILL_SOLID;
			rd.CullMode = D3D11_CULL_NONE;
			rd.DepthClipEnable = TRUE;
			g_device->CreateRasterizerState(&rd, &g_raster);

			D3D11_DEPTH_STENCIL_DESC dd{};
			dd.DepthEnable = FALSE;
			dd.StencilEnable = FALSE;
			g_device->CreateDepthStencilState(&dd, &g_depth);

			D3D11_BUFFER_DESC cbd{};
			cbd.ByteWidth = sizeof(Params);
			cbd.Usage = D3D11_USAGE_DYNAMIC;
			cbd.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
			cbd.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
			g_device->CreateBuffer(&cbd, nullptr, &g_params);
			g_blurBroken = !g_psCopy || !g_psBlur;

			const bool ok = g_vs && g_ps && g_blend && g_sampler && g_raster && g_depth && g_params;
			if (!ok) {
				g_initError = "creating shaders / states failed";
			}
			logger::info("overlay draw: renderer {}", ok ? "ready" : "failed to initialize");
			g_initFailed = !ok;
			return ok;
		}

		bool EnsureTexture(UINT a_w, UINT a_h, bool a_srgb)
		{
			if (g_texture && g_texW == a_w && g_texH == a_h && g_texSrgb == a_srgb) {
				return true;
			}
			SafeRelease(g_srv);
			SafeRelease(g_texture);
			D3D11_TEXTURE2D_DESC td{};
			td.Width = a_w;
			td.Height = a_h;
			td.MipLevels = 1;
			td.ArraySize = 1;
			// Garry's Mod's bytes are B, G, R, A (Direct3D 9 A8R8G8B8).
			td.Format = a_srgb ? DXGI_FORMAT_B8G8R8A8_UNORM_SRGB : DXGI_FORMAT_B8G8R8A8_UNORM;
			td.SampleDesc.Count = 1;
			td.Usage = D3D11_USAGE_DYNAMIC;
			td.BindFlags = D3D11_BIND_SHADER_RESOURCE;
			td.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
			if (FAILED(g_device->CreateTexture2D(&td, nullptr, &g_texture)) || FAILED(g_device->CreateShaderResourceView(g_texture, nullptr, &g_srv))) {
				SafeRelease(g_texture);
				Error(fmt::format("texture {}x{} ({}) creation failed", a_w, a_h, a_srgb ? "sRGB" : "plain"));
				return false;
			}
			g_texW = a_w;
			g_texH = a_h;
			g_texSrgb = a_srgb;
			g_havePicture = false;
			logger::info("overlay draw: texture {}x{} ({})", a_w, a_h, a_srgb ? "sRGB: decoded to linear when sampled" : "plain");
			return true;
		}

		// Takes the newest picture, if there is one, into the texture.
		void Upload()
		{
			uint32_t slot = 0;
			if (!Overlay::Take(&slot)) {
				return;
			}
			++g_taken;
			const auto hdr = Overlay::Header(slot);
			if (hdr.width == 0 || hdr.height == 0 || hdr.width > GMSKY_OV_MAX_W || hdr.height > GMSKY_OV_MAX_H || hdr.format != GMSKY_OV_FMT_BGRA8) {
				Error(fmt::format("picture {}x{} format {} can't be used", hdr.width, hdr.height, hdr.format));
				return;
			}
			const bool srgb = g_linear == 1 && (hdr.flags & GMSKY_OVF_SRGB);
			if (!EnsureTexture(hdr.width, hdr.height, srgb)) {
				return;
			}
			const auto t0 = Qpc();
			D3D11_MAPPED_SUBRESOURCE mapped{};
			if (FAILED(g_context->Map(g_texture, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
				Error("Map of the texture failed");
				return;
			}
			const auto* src = Overlay::Pixels(slot);
			const UINT rowBytes = hdr.row_bytes ? hdr.row_bytes : hdr.width * 4;
			auto* dst = static_cast<uint8_t*>(mapped.pData);
			if (mapped.RowPitch == rowBytes) {
				std::memcpy(dst, src, std::size_t(rowBytes) * hdr.height);
			} else {
				for (UINT y = 0; y < hdr.height; ++y) {
					std::memcpy(dst + std::size_t(y) * mapped.RowPitch, src + std::size_t(y) * rowBytes, std::size_t(hdr.width) * 4);
				}
			}
			g_context->Unmap(g_texture, 0);
			const auto dt = uint64_t(Qpc() - t0);
			g_uploadTicks += dt;
			Max(g_uploadMaxTicks, dt);
			g_havePicture = true;
			g_pictureQpc = hdr.capture_qpc;
			g_pictureFrame = hdr.frame;
			g_pictureSlot = slot;
			g_pictureFlags = hdr.flags;
		}

		// The back buffer's render target view: an sRGB view (linear blending) if the format allows
		// it, else a plain one (approximate). Decided once, logged.
		ID3D11RenderTargetView* BackBufferView(IDXGISwapChain* a_swapChain, D3D11_TEXTURE2D_DESC& a_desc)
		{
			ID3D11Texture2D* bb = nullptr;
			if (FAILED(a_swapChain->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&bb)))) {
				Error("swap chain GetBuffer failed");
				return nullptr;
			}
			bb->GetDesc(&a_desc);
			g_bbFormat = a_desc.Format;
			g_bbW = a_desc.Width;
			g_bbH = a_desc.Height;
			DXGI_FORMAT srgb = DXGI_FORMAT_UNKNOWN;
			switch (a_desc.Format) {
			case DXGI_FORMAT_R8G8B8A8_UNORM:
			case DXGI_FORMAT_R8G8B8A8_TYPELESS:
			case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
				srgb = DXGI_FORMAT_R8G8B8A8_UNORM_SRGB;
				break;
			case DXGI_FORMAT_B8G8R8A8_UNORM:
			case DXGI_FORMAT_B8G8R8A8_TYPELESS:
			case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB:
				srgb = DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
				break;
			default:
				break;
			}
			ID3D11RenderTargetView* rtv = nullptr;
			if (g_linear != 0 && srgb != DXGI_FORMAT_UNKNOWN) {
				D3D11_RENDER_TARGET_VIEW_DESC vd{};
				vd.Format = srgb;
				vd.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2D;
				const auto hr = g_device->CreateRenderTargetView(bb, &vd, &rtv);
				if (g_linear == -1) {
					g_linear = SUCCEEDED(hr) ? 1 : 0;
					logger::info("overlay draw: Skyrim's back buffer {}x{} format {}: {}", a_desc.Width, a_desc.Height, int(a_desc.Format),
						SUCCEEDED(hr) ? "sRGB view made, blending in linear light" :
										fmt::format("no sRGB view (0x{:08X}): approximate blending (as SkyCraft)", uint32_t(hr)));
				}
			} else if (g_linear == -1) {
				g_linear = 0;
				logger::info("overlay draw: Skyrim's back buffer {}x{} format {} has no sRGB variant: approximate blending (as SkyCraft)",
					a_desc.Width, a_desc.Height, int(a_desc.Format));
			}
			if (!rtv && FAILED(g_device->CreateRenderTargetView(bb, nullptr, &rtv))) {
				rtv = nullptr;
				Error(fmt::format("back buffer render target view failed (format {})", int(a_desc.Format)));
			}
			SafeRelease(bb);
			return rtv;
		}

		void SetParams(float a_blurOn, float a_dx, float a_dy)
		{
			D3D11_MAPPED_SUBRESOURCE m{};
			if (SUCCEEDED(g_context->Map(g_params, 0, D3D11_MAP_WRITE_DISCARD, 0, &m))) {
				*static_cast<Params*>(m.pData) = Params{ a_blurOn, kBlurFull, { a_dx, a_dy } };
				g_context->Unmap(g_params, 0);
			}
		}

		void ReleaseTarget(Target& a_t)
		{
			SafeRelease(a_t.srv);
			SafeRelease(a_t.rtv);
			SafeRelease(a_t.tex);
			a_t.w = a_t.h = 0;
		}

		bool MakeTarget(Target& a_t, UINT a_w, UINT a_h)
		{
			D3D11_TEXTURE2D_DESC td{};
			td.Width = std::max(1u, a_w);
			td.Height = std::max(1u, a_h);
			td.MipLevels = 1;
			td.ArraySize = 1;
			td.Format = DXGI_FORMAT_R16G16B16A16_FLOAT;  // linear light, no banding
			td.SampleDesc.Count = 1;
			td.Usage = D3D11_USAGE_DEFAULT;
			td.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
			if (FAILED(g_device->CreateTexture2D(&td, nullptr, &a_t.tex)) || FAILED(g_device->CreateRenderTargetView(a_t.tex, nullptr, &a_t.rtv)) ||
				FAILED(g_device->CreateShaderResourceView(a_t.tex, nullptr, &a_t.srv))) {
				ReleaseTarget(a_t);
				return false;
			}
			a_t.w = td.Width;
			a_t.h = td.Height;
			return true;
		}

		// The blur's textures for this back buffer (remade when its size or format changes).
		// False: no blur (why logged once).
		bool EnsureBlur(const D3D11_TEXTURE2D_DESC& a_bb)
		{
			if (g_blurBroken) {
				return false;
			}
			if (g_sceneCopy && g_blurW == a_bb.Width && g_blurH == a_bb.Height && g_blurFormat == a_bb.Format) {
				return true;
			}
			SafeRelease(g_sceneSrv);
			SafeRelease(g_sceneCopy);
			ReleaseTarget(g_half);
			ReleaseTarget(g_quarterA);
			ReleaseTarget(g_quarterB);
			DXGI_FORMAT typeless = DXGI_FORMAT_UNKNOWN, view = DXGI_FORMAT_UNKNOWN;
			switch (a_bb.Format) {
			case DXGI_FORMAT_R8G8B8A8_UNORM:
			case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
			case DXGI_FORMAT_R8G8B8A8_TYPELESS:
				typeless = DXGI_FORMAT_R8G8B8A8_TYPELESS;
				view = g_linear == 1 ? DXGI_FORMAT_R8G8B8A8_UNORM_SRGB : DXGI_FORMAT_R8G8B8A8_UNORM;
				break;
			case DXGI_FORMAT_B8G8R8A8_UNORM:
			case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB:
			case DXGI_FORMAT_B8G8R8A8_TYPELESS:
				typeless = DXGI_FORMAT_B8G8R8A8_TYPELESS;
				view = g_linear == 1 ? DXGI_FORMAT_B8G8R8A8_UNORM_SRGB : DXGI_FORMAT_B8G8R8A8_UNORM;
				break;
			default:
				break;
			}
			if (typeless == DXGI_FORMAT_UNKNOWN || a_bb.SampleDesc.Count != 1) {
				g_blurBroken = true;
				logger::error("overlay draw: no blur (back buffer format {}, {} samples)", int(a_bb.Format), a_bb.SampleDesc.Count);
				return false;
			}
			D3D11_TEXTURE2D_DESC td{};
			td.Width = a_bb.Width;
			td.Height = a_bb.Height;
			td.MipLevels = 1;
			td.ArraySize = 1;
			td.Format = typeless;
			td.SampleDesc.Count = 1;
			td.Usage = D3D11_USAGE_DEFAULT;
			td.BindFlags = D3D11_BIND_SHADER_RESOURCE;
			D3D11_SHADER_RESOURCE_VIEW_DESC sd{};
			sd.Format = view;
			sd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
			sd.Texture2D.MipLevels = 1;
			if (FAILED(g_device->CreateTexture2D(&td, nullptr, &g_sceneCopy)) || FAILED(g_device->CreateShaderResourceView(g_sceneCopy, &sd, &g_sceneSrv)) ||
				!MakeTarget(g_half, a_bb.Width / 2, a_bb.Height / 2) || !MakeTarget(g_quarterA, a_bb.Width / 4, a_bb.Height / 4) ||
				!MakeTarget(g_quarterB, a_bb.Width / 4, a_bb.Height / 4)) {
				g_blurBroken = true;
				logger::error("overlay draw: no blur (making its textures failed)");
				return false;
			}
			g_blurW = a_bb.Width;
			g_blurH = a_bb.Height;
			g_blurFormat = a_bb.Format;
			logger::info("overlay draw: blur textures for {}x{} (quarter size {}x{}, {} passes)", a_bb.Width, a_bb.Height, g_quarterA.w,
				g_quarterA.h, 2 + 2 * kBlurIterations);
			return true;
		}

		// Skyrim's frame, copied, shrunk to a quarter and blurred, ends up in g_quarterA.
		// Graphics settings are saved and restored by the caller.
		bool Blur(IDXGISwapChain* a_swapChain)
		{
			ID3D11Texture2D* bb = nullptr;
			if (FAILED(a_swapChain->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&bb)))) {
				return false;
			}
			const auto t0 = Qpc();
			g_context->CopyResource(g_sceneCopy, bb);
			SafeRelease(bb);
			ID3D11ShaderResourceView* none[2]{};
			const float factor[4]{};
			g_context->OMSetBlendState(nullptr, factor, 0xFFFFFFFF);  // passes replace, no blending
			auto pass = [&](Target& a_dst, ID3D11PixelShader* a_ps, ID3D11ShaderResourceView* a_src, float a_dx, float a_dy) {
				g_context->PSSetShaderResources(0, 2, none);  // nothing is read and written at once
				g_context->OMSetRenderTargets(1, &a_dst.rtv, nullptr);
				const D3D11_VIEWPORT vp{ 0, 0, float(a_dst.w), float(a_dst.h), 0, 1 };
				g_context->RSSetViewports(1, &vp);
				SetParams(0.0f, a_dx, a_dy);
				g_context->PSSetShader(a_ps, nullptr, 0);
				g_context->PSSetShaderResources(1, 1, &a_src);
				g_context->Draw(3, 0);
			};
			pass(g_half, g_psCopy, g_sceneSrv, 0, 0);
			pass(g_quarterA, g_psCopy, g_half.srv, 0, 0);
			for (int i = 0; i < kBlurIterations; ++i) {
				pass(g_quarterB, g_psBlur, g_quarterA.srv, 1.0f / float(g_quarterA.w), 0);
				pass(g_quarterA, g_psBlur, g_quarterB.srv, 0, 1.0f / float(g_quarterA.h));
			}
			g_context->PSSetShaderResources(0, 2, none);
			g_blurTicks += uint64_t(Qpc() - t0);
			++g_blurred;
			return true;
		}

		void DrawPicture(IDXGISwapChain* a_swapChain)
		{
			D3D11_TEXTURE2D_DESC bbDesc{};
			ID3D11RenderTargetView* rtv = BackBufferView(a_swapChain, bbDesc);
			if (!rtv) {
				return;
			}
			// A texture made before the blending mode was known is remade on the next picture.
			if (g_texture && g_texSrgb != (g_linear == 1)) {
				SafeRelease(rtv);
				g_havePicture = false;
				SafeRelease(g_srv);
				SafeRelease(g_texture);
				return;
			}
			const auto t0 = Qpc();
			// Save the settings we touch (as SkyCraft).
			ID3D11RenderTargetView* oldRtv[D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT]{};
			ID3D11DepthStencilView* oldDsv = nullptr;
			ID3D11BlendState* oldBlend = nullptr;
			float oldFactor[4]{};
			UINT oldMask = 0;
			ID3D11RasterizerState* oldRaster = nullptr;
			ID3D11DepthStencilState* oldDepth = nullptr;
			UINT oldStencil = 0;
			D3D11_VIEWPORT oldVps[D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE]{};
			UINT oldVpCount = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
			D3D11_PRIMITIVE_TOPOLOGY oldTopo{};
			ID3D11InputLayout* oldLayout = nullptr;
			ID3D11VertexShader* oldVs = nullptr;
			ID3D11PixelShader* oldPs = nullptr;
			ID3D11ShaderResourceView* oldSrv[2]{};
			ID3D11SamplerState* oldSampler = nullptr;
			ID3D11Buffer* oldCb = nullptr;
			g_context->OMGetRenderTargets(D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT, oldRtv, &oldDsv);
			g_context->OMGetBlendState(&oldBlend, oldFactor, &oldMask);
			g_context->RSGetState(&oldRaster);
			g_context->OMGetDepthStencilState(&oldDepth, &oldStencil);
			g_context->RSGetViewports(&oldVpCount, oldVps);
			g_context->IAGetPrimitiveTopology(&oldTopo);
			g_context->IAGetInputLayout(&oldLayout);
			g_context->VSGetShader(&oldVs, nullptr, nullptr);
			g_context->PSGetShader(&oldPs, nullptr, nullptr);
			g_context->PSGetShaderResources(0, 2, oldSrv);
			g_context->PSGetSamplers(0, 1, &oldSampler);
			g_context->PSGetConstantBuffers(0, 1, &oldCb);

			// Settings shared by the blur passes and the drawing.
			g_context->OMSetDepthStencilState(g_depth, 0);
			g_context->RSSetState(g_raster);
			g_context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
			g_context->IASetInputLayout(nullptr);
			g_context->VSSetShader(g_vs, nullptr, 0);
			g_context->PSSetSamplers(0, 1, &g_sampler);
			g_context->PSSetConstantBuffers(0, 1, &g_params);
			// Blur behind see-through parts, if Garry's Mod asks for it (gmsky_overlay_blur).
			const bool blur = (g_pictureFlags & GMSKY_OVF_BLUR) && EnsureBlur(bbDesc) && Blur(a_swapChain);

			D3D11_VIEWPORT vp{ 0, 0, float(bbDesc.Width), float(bbDesc.Height), 0, 1 };
			const float factor[4]{};
			g_context->OMSetRenderTargets(1, &rtv, nullptr);
			g_context->OMSetBlendState(g_blend, factor, 0xFFFFFFFF);
			g_context->RSSetViewports(1, &vp);
			SetParams(blur ? 1.0f : 0.0f, 0, 0);
			g_context->PSSetShader(g_ps, nullptr, 0);
			ID3D11ShaderResourceView* srvs[2]{ g_srv, blur ? g_quarterA.srv : nullptr };
			g_context->PSSetShaderResources(0, 2, srvs);
			g_context->Draw(3, 0);
			ID3D11ShaderResourceView* none[2]{};
			g_context->PSSetShaderResources(0, 2, none);

			// Put everything back.
			g_context->OMSetRenderTargets(D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT, oldRtv, oldDsv);
			g_context->OMSetBlendState(oldBlend, oldFactor, oldMask);
			g_context->OMSetDepthStencilState(oldDepth, oldStencil);
			g_context->RSSetState(oldRaster);
			g_context->RSSetViewports(oldVpCount, oldVps);
			g_context->IASetPrimitiveTopology(oldTopo);
			g_context->IASetInputLayout(oldLayout);
			g_context->VSSetShader(oldVs, nullptr, 0);
			g_context->PSSetShader(oldPs, nullptr, 0);
			g_context->PSSetShaderResources(0, 2, oldSrv);
			g_context->PSSetSamplers(0, 1, &oldSampler);
			g_context->PSSetConstantBuffers(0, 1, &oldCb);
			for (auto*& r : oldRtv) {
				SafeRelease(r);
			}
			SafeRelease(oldDsv);
			SafeRelease(oldBlend);
			SafeRelease(oldRaster);
			SafeRelease(oldDepth);
			SafeRelease(oldLayout);
			SafeRelease(oldVs);
			SafeRelease(oldPs);
			for (auto*& s : oldSrv) {
				SafeRelease(s);
			}
			SafeRelease(oldSampler);
			SafeRelease(oldCb);
			SafeRelease(rtv);
			g_drawTicks += uint64_t(Qpc() - t0);
			++g_drawn;
		}

		// ---- Snapshot (gmsky_overlay_shot): Skyrim's frame before and after our picture, and the
		// picture, as raw files for tools\skyrim_shot_report.py. Format: "GSHT", width, height, DXGI
		// format (u32 each), then the rows, width * 4 bytes each, top to bottom. ----
		uint32_t g_shotSeen = UINT32_MAX;

		std::filesystem::path ShotPath(uint32_t a_n, const char* a_what)
		{
			return std::filesystem::path(GMSKY_PROJECT_ROOT) / "logs" / "overlay" / fmt::format("skyrim_shot_{}_{}.bin", a_n, a_what);
		}

		bool WriteShot(const std::filesystem::path& a_path, UINT a_w, UINT a_h, UINT a_format, const uint8_t* a_rows, UINT a_pitch)
		{
			std::error_code ec;
			std::filesystem::create_directories(a_path.parent_path(), ec);
			FILE* f = nullptr;
			if (_wfopen_s(&f, a_path.c_str(), L"wb") != 0 || !f) {
				return false;
			}
			const uint32_t head[4] = { 0x54485347u /* "GSHT" */, a_w, a_h, a_format };
			bool ok = fwrite(head, sizeof(head), 1, f) == 1;
			for (UINT y = 0; ok && y < a_h; ++y) {
				ok = fwrite(a_rows + std::size_t(y) * a_pitch, std::size_t(a_w) * 4, 1, f) == 1;
			}
			fclose(f);
			return ok;
		}

		// Copies the back buffer to a CPU-readable texture and writes it.
		bool SaveBackBuffer(IDXGISwapChain* a_swapChain, const std::filesystem::path& a_path)
		{
			ID3D11Texture2D* bb = nullptr;
			if (FAILED(a_swapChain->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&bb)))) {
				return false;
			}
			D3D11_TEXTURE2D_DESC d{};
			bb->GetDesc(&d);
			if (d.SampleDesc.Count != 1) {
				SafeRelease(bb);
				return false;
			}
			D3D11_TEXTURE2D_DESC sd = d;
			sd.Usage = D3D11_USAGE_STAGING;
			sd.BindFlags = 0;
			sd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
			sd.MiscFlags = 0;
			ID3D11Texture2D* staging = nullptr;
			bool ok = SUCCEEDED(g_device->CreateTexture2D(&sd, nullptr, &staging));
			if (ok) {
				g_context->CopyResource(staging, bb);
				D3D11_MAPPED_SUBRESOURCE m{};
				ok = SUCCEEDED(g_context->Map(staging, 0, D3D11_MAP_READ, 0, &m));
				if (ok) {
					ok = WriteShot(a_path, d.Width, d.Height, d.Format, static_cast<const uint8_t*>(m.pData), m.RowPitch);
					g_context->Unmap(staging, 0);
				}
			}
			SafeRelease(staging);
			SafeRelease(bb);
			return ok;
		}

		void OnPresent(IDXGISwapChain* a_swapChain)
		{
			++g_presents;
			g_presentThread = GetCurrentThreadId();
			if (!Overlay::IsOpen() || !InitResources(a_swapChain)) {
				return;
			}
			if (Input::SkyrimMenuOpen()) {
				++g_skipMenu;
				return;
			}
			if (!Puppet::IsActive()) {
				++g_skipPuppet;
				return;
			}
			if (g_linear == -1) {
				// The blending mode decides the texture's format: find it out first.
				D3D11_TEXTURE2D_DESC d{};
				if (auto* rtv = BackBufferView(a_swapChain, d)) {
					rtv->Release();
				}
			}
			Upload();
			if (!g_havePicture) {
				++g_skipNoPicture;
				return;
			}
			const double age = QpcMs(Qpc() - g_pictureQpc);
			if (age > kStaleMs) {
				++g_skipStale;
				return;
			}
			g_ageSumUs += uint64_t(age * 1000.0);
			Max(g_ageMaxUs, uint64_t(age * 1000.0));
			// A new snapshot request (an old one from before Skyrim started is ignored).
			const uint32_t req = Overlay::ShotRequest();
			if (g_shotSeen == UINT32_MAX) {
				g_shotSeen = req;
			}
			const bool shot = req != g_shotSeen;
			g_shotSeen = req;
			bool before = false;
			if (shot) {
				before = SaveBackBuffer(a_swapChain, ShotPath(req, "before"));
			}
			DrawPicture(a_swapChain);
			if (shot) {
				const bool after = SaveBackBuffer(a_swapChain, ShotPath(req, "after"));
				const auto hdr = Overlay::Header(g_pictureSlot);
				const bool pic = hdr.frame == g_pictureFrame &&
				                 WriteShot(ShotPath(req, "gmod"), hdr.width, hdr.height, DXGI_FORMAT_B8G8R8A8_UNORM, Overlay::Pixels(g_pictureSlot), hdr.row_bytes);
				logger::info("overlay draw: snapshot {} saved (before {}, after {}, Garry's Mod picture frame {} {}; blending {}, blur {}, picture {:.1f} ms old)",
					req, before ? "ok" : "FAILED", after ? "ok" : "FAILED", g_pictureFrame, pic ? "ok" : "FAILED (already replaced)",
					g_linear == 1 ? "linear" : "approximate", (g_pictureFlags & GMSKY_OVF_BLUR) && !g_blurBroken ? "on" : "off", age);
			}
		}

		HRESULT WINAPI PresentHook(IDXGISwapChain* a_swapChain, UINT a_sync, UINT a_flags)
		{
			try {
				OnPresent(a_swapChain);
			} catch (const std::exception& e) {
				Error(fmt::format("exception: {}", e.what()));
			} catch (...) {
				Error("unknown exception");
			}
			return g_originalPresent(a_swapChain, a_sync, a_flags);
		}
	}

	void Install()
	{
		auto* window = RE::BSGraphics::Renderer::GetCurrentRenderWindow();
		auto* swapChain = window ? reinterpret_cast<IDXGISwapChain*>(window->swapChain) : nullptr;
		if (!swapChain) {
			logger::error("overlay draw: no swap chain yet, Present not hooked");
			return;
		}
		auto** vtable = *reinterpret_cast<void***>(swapChain);
		DWORD oldProtect = 0;
		VirtualProtect(&vtable[8], sizeof(void*), PAGE_EXECUTE_READWRITE, &oldProtect);
		g_originalPresent = reinterpret_cast<PresentFn>(vtable[8]);
		vtable[8] = reinterpret_cast<void*>(&PresentHook);
		VirtualProtect(&vtable[8], sizeof(void*), oldProtect, &oldProtect);
		logger::info("overlay draw: Present hooked (swap chain entry 8)");
	}

	void LogSecond()
	{
		const auto presents = g_presents.exchange(0), taken = g_taken.exchange(0), drawn = g_drawn.exchange(0);
		const auto menu = g_skipMenu.exchange(0), puppet = g_skipPuppet.exchange(0), none = g_skipNoPicture.exchange(0);
		const auto stale = g_skipStale.exchange(0), errors = g_skipError.exchange(0);
		const auto upTicks = g_uploadTicks.exchange(0), upMax = g_uploadMaxTicks.exchange(0), drawTicks = g_drawTicks.exchange(0);
		const auto ageSum = g_ageSumUs.exchange(0), ageMax = g_ageMaxUs.exchange(0);
		const auto blurred = g_blurred.exchange(0), blurTicks = g_blurTicks.exchange(0);
		std::string err;
		{
			std::lock_guard lock(g_errorMutex);
			err.swap(g_lastError);
		}
		logger::info("overlay draw: {} presents (thread {}, game thread {}), {} pictures taken, {} drawn; not drawn: {} menu, {} puppet off, "
			"{} no picture yet, {} stale (> {:.0f} ms), {} errors{}; upload {:.2f} ms avg {:.2f} max, draw {:.3f} ms avg; picture age {:.1f} ms avg {:.1f} max "
			"(newest frame {}); blending {}; blur: {} frames, {:.3f} ms avg (CPU), {}; back buffer {}x{} format {}{}",
			presents, g_presentThread.load(), GetCurrentThreadId(), taken, drawn, menu, puppet, none, stale, kStaleMs, errors,
			err.empty() ? "" : " (last: " + err + ")",
			taken ? QpcMs(int64_t(upTicks)) / double(taken) : 0.0, QpcMs(int64_t(upMax)), drawn ? QpcMs(int64_t(drawTicks)) / double(drawn) : 0.0,
			drawn ? double(ageSum) / 1000.0 / double(drawn) : 0.0, double(ageMax) / 1000.0, g_pictureFrame,
			g_linear == 1 ? "linear (sRGB)" : g_linear == 0 ? "approximate" : "not decided yet",
			blurred, blurred ? QpcMs(int64_t(blurTicks)) / double(blurred) : 0.0,
			g_blurBroken ? "UNAVAILABLE" : (g_pictureFlags & GMSKY_OVF_BLUR) ? "asked for" : "off (gmsky_overlay_blur 0)",
			g_bbW.load(), g_bbH.load(), g_bbFormat.load(),
			g_initError.empty() ? "" : "; init error: " + g_initError);
	}
}
