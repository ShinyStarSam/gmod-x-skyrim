// Geometry route (DESIGN.md §12, "Garry's Mod's things drawn inside Skyrim"): Skyrim draws Garry's
// Mod's objects itself, inside its own frame. Adapted from SkyCraft (MIT), skse/src/WorldRender.cpp:
// a hook right after the call to Main::RenderWorld (REL::ID(36559) + 0x85E, checked to be a call to
// REL::ID(107142) before patching). There Skyrim's 3D scene is complete and its post-processing
// (tone mapping, bloom, TAA) hasn't run, so what we draw into its HDR scene target with its main
// depth buffer bound is depth-tested against Skyrim's world, writes Skyrim's depth, and goes
// through Skyrim's post-processing like its own objects. Motion vectors are written too, so
// Skyrim's TAA doesn't smear it. Camera: Skyrim's own world -> clip matrix of this frame, re-based
// on the camera position (SkyCraft's RenderPasses); every graphics setting is saved and put back
// (SkyCraft's StateBackup).
//
// G1: a fixed test cube (kTestCube; off since G3a). G3a: the models, textures and newest frame
// from the render block (Render.cpp) are uploaded and drawn. Each model is bent on the graphics
// card by its object's bone matrices (up to 4 bones per corner; what Source itself does) and
// placed at origin - camera + position / scale (§5; the subtraction in double precision here).
// Fixed light for now (Skyrim's light is G5). Differs from SkyCraft: SkyCraft receives posed
// triangles every frame; we receive each model once and its bone matrices every frame.
#include "World.h"

#include "Draw.h"
#include "Input.h"
#include "Link.h"
#include "Render.h"

#include <d3d11.h>
#include <d3dcompiler.h>

#include <mutex>

namespace gmsky::World
{
	namespace
	{
		// G1's test cube, replaced by Garry's Mod's objects in G3a.
		constexpr bool kTestCube = false;
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
	float4 offset;                    // the test cube's centre, camera-relative
	float4 lightDir;                  // towards the light (Skyrim axes)
};
cbuffer Object : register(b1)
{
	float4 objOffset;  // xyz: the anchor (Skyrim's origin) relative to the camera, Skyrim units; w: Skyrim units per Garry's Mod unit
	uint4  objInfo;    // x: the object's first matrix in boneRows (x 3 rows); y: GMSKY_RND_MAT_* of this part
	float4 objColor;   // the object's tint
	float4 objAlpha;   // x: alpha reference (alpha-tested parts)
};
StructuredBuffer<float4> boneRows : register(t0);  // every matrix of the frame, 3 rows each
Texture2D albedo : register(t1);
SamplerState albedoSampler : register(s0);

struct PSOut
{
	float4 color : SV_Target0;
	float4 motion : SV_Target1;  // Skyrim's motion vectors (for its TAA), when bound
};

float2 Motion(float4 cur, float4 prev)
{
	return float2(-0.5, 0.5) * (cur.xy / cur.w - prev.xy / prev.w);  // Skyrim's convention: previous uv - current uv
}

// ---- G1's cube ----
struct CubeIn
{
	float3 pos : POSITION;
	float3 normal : NORMAL;
};
struct CubeOut
{
	float4 pos : SV_Position;
	float3 normal : NORMAL;
	float4 curClip : TEXCOORD0;
	float4 prevClip : TEXCOORD1;
};

CubeOut CubeVS(CubeIn i)
{
	CubeOut o;
	float3  rel = offset.xyz + i.pos;
	o.pos = mul(viewProj, float4(rel, 1.0));
	o.normal = i.normal;
	o.curClip = o.pos;
	o.prevClip = mul(prevViewProj, float4(rel, 1.0));
	return o;
}

PSOut CubePS(CubeOut i)
{
	PSOut po;
	po.motion = float4(Motion(i.curClip, i.prevClip), 0.0, 1.0);
	float lit = 0.35 + 0.65 * saturate(dot(normalize(i.normal), lightDir.xyz));
	po.color = float4(lit.xxx, 1.0);
	return po;
}

// ---- Garry's Mod's models ----
struct ModelIn
{
	float3 pos : POSITION;
	float3 normal : NORMAL;
	float2 uv : TEXCOORD0;
	uint4  bones : BLENDINDICES;
	float4 weights : BLENDWEIGHT;
};
struct ModelOut
{
	float4 pos : SV_Position;
	float3 normal : NORMAL;
	float2 uv : TEXCOORD0;
	float4 curClip : TEXCOORD1;
	float4 prevClip : TEXCOORD2;
};

ModelOut ModelVS(ModelIn i)
{
	float3 p = 0, n = 0;
	float4 hp = float4(i.pos, 1.0);
	[unroll] for (int k = 0; k < 4; ++k) {
		float w = i.weights[k];
		if (w > 0.0) {
			uint   b = (objInfo.x + i.bones[k]) * 3;
			float4 r0 = boneRows[b], r1 = boneRows[b + 1], r2 = boneRows[b + 2];
			p += w * float3(dot(r0, hp), dot(r1, hp), dot(r2, hp));
			n += w * float3(dot(r0.xyz, i.normal), dot(r1.xyz, i.normal), dot(r2.xyz, i.normal));
		}
	}
	ModelOut o;
	float3   rel = objOffset.xyz + p * objOffset.w;  // Garry's Mod and Skyrim share their axes (Z up)
	o.pos = mul(viewProj, float4(rel, 1.0));
	o.normal = n;
	o.uv = i.uv;
	o.curClip = o.pos;
	o.prevClip = mul(prevViewProj, float4(rel, 1.0));  // the object's own motion isn't in it yet
	return o;
}

PSOut ModelPS(ModelOut i)
{
	PSOut  po;
	float4 c = albedo.Sample(albedoSampler, i.uv) * objColor;
	if (objInfo.y & 0x4u) {  // GMSKY_RND_MAT_ALPHATEST
		clip(c.a - objAlpha.x);
	}
	float3 n = normalize(i.normal);
	float  lit = 0.35 + 0.65 * saturate(dot(n, lightDir.xyz));
	po.color = float4(c.rgb * lit, 1.0);
	po.motion = float4(Motion(i.curClip, i.prevClip), 0.0, 1.0);
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

		struct ObjectConstants
		{
			float offset[4];
			uint32_t info[4];
			float color[4];
			float alpha[4];
		};
		static_assert(sizeof(ObjectConstants) == 64);

		struct CubeVertex
		{
			float pos[3];
			float normal[3];
		};

		ID3D11Device* g_device = nullptr;
		ID3D11VertexShader* g_cubeVs = nullptr;
		ID3D11PixelShader* g_cubePs = nullptr;
		ID3D11InputLayout* g_cubeLayout = nullptr;
		ID3D11VertexShader* g_modelVs = nullptr;
		ID3D11PixelShader* g_modelPs = nullptr;
		ID3D11InputLayout* g_modelLayout = nullptr;
		ID3D11Buffer* g_frameCb = nullptr;
		ID3D11Buffer* g_objectCb = nullptr;
		ID3D11Buffer* g_cubeVb = nullptr;
		ID3D11RasterizerState* g_raster = nullptr;
		ID3D11DepthStencilState* g_depthWrite = nullptr;     // standard Z (0 near, 1 far)
		ID3D11DepthStencilState* g_depthWriteRev = nullptr;  // reversed Z
		ID3D11BlendState* g_solidBlend = nullptr;
		ID3D11SamplerState* g_sampler = nullptr;
		ID3D11Texture2D* g_whiteTex = nullptr;
		ID3D11ShaderResourceView* g_whiteSrv = nullptr;
		ID3D11Buffer* g_boneBuf = nullptr;  // the newest frame's matrices (rows as float4)
		ID3D11ShaderResourceView* g_boneSrv = nullptr;
		UINT g_boneCapacity = 0;  // float4 rows
		bool g_initFailed = false;
		bool g_hooked = false;
		bool g_targetsChecked = false, g_targetsOk = false;

		double g_prevW2C[4][4]{};
		bool g_havePrevW2C = false;

		// The cube's place in Skyrim's world (render thread); placed again after a load.
		std::atomic<bool> g_placeCube{ true };
		double g_cube[3]{};
		bool g_cubePlaced = false;

		// Garry's Mod's models and textures on the graphics card, by the ids Garry's Mod gave them.
		struct GpuModel
		{
			uint64_t serial = 0;
			ID3D11Buffer* vb = nullptr;
			ID3D11Buffer* ib = nullptr;
			std::vector<GmskyRndPart> parts;
			uint32_t bones = 0, triangles = 0;
		};
		struct GpuTexture
		{
			uint64_t serial = 0;
			ID3D11Texture2D* tex = nullptr;
			ID3D11ShaderResourceView* srv = nullptr;
		};
		std::unordered_map<uint32_t, GpuModel> g_gpuModels;
		std::unordered_map<uint32_t, GpuTexture> g_gpuTextures;

		// Counters since the last LogSecond (render thread writes, game thread reads).
		std::atomic<uint32_t> g_calls{ 0 }, g_drawn{ 0 }, g_skipMenu{ 0 }, g_skipNoCamera{ 0 }, g_skipTargets{ 0 }, g_skipInit{ 0 };
		std::atomic<int64_t> g_drawTicks{ 0 }, g_drawMaxTicks{ 0 };

		// What the last drawn frame saw, for the once-a-second line (under g_seenLock).
		struct Seen
		{
			bool cubeValid = false;
			double cam[3]{};
			double cube[3]{};
			bool onScreen = false;
			float screenX = 0, screenY = 0, distance = 0;
			bool reversed = false;
			const char* depthFrom = "";
			// G3a
			uint64_t objectsDrawn = 0, skippedEpoch = 0, skippedNoOrigin = 0, skippedUnknown = 0, skippedBones = 0;
			uint64_t triangles = 0, drawCalls = 0, framesDrawn = 0;
			uint64_t modelUploads = 0, textureUploads = 0, uploadFailures = 0;
			double uploadMsMax = 0;
			size_t gpuModels = 0, gpuTextures = 0;
			uint64_t gmodFrame = 0;
			uint32_t frameEpoch = 0, originEpoch = 0;
			bool firstValid = false;
			double first[3]{};  // the first object's first matrix's translation, in Skyrim's world
			bool firstOnScreen = false;
			float firstX = 0, firstY = 0, firstDistance = 0;
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
			ID3DBlob *cubeVsBlob = nullptr, *cubePsBlob = nullptr, *modelVsBlob = nullptr, *modelPsBlob = nullptr;
			if (!Compile("CubeVS", "vs_5_0", &cubeVsBlob) || !Compile("CubePS", "ps_5_0", &cubePsBlob) || !Compile("ModelVS", "vs_5_0", &modelVsBlob) ||
				!Compile("ModelPS", "ps_5_0", &modelPsBlob)) {
				Release(cubeVsBlob);
				Release(cubePsBlob);
				Release(modelVsBlob);
				g_initFailed = true;
				return false;
			}
			a_device->CreateVertexShader(cubeVsBlob->GetBufferPointer(), cubeVsBlob->GetBufferSize(), nullptr, &g_cubeVs);
			a_device->CreatePixelShader(cubePsBlob->GetBufferPointer(), cubePsBlob->GetBufferSize(), nullptr, &g_cubePs);
			a_device->CreateVertexShader(modelVsBlob->GetBufferPointer(), modelVsBlob->GetBufferSize(), nullptr, &g_modelVs);
			a_device->CreatePixelShader(modelPsBlob->GetBufferPointer(), modelPsBlob->GetBufferSize(), nullptr, &g_modelPs);
			const D3D11_INPUT_ELEMENT_DESC cubeElements[] = {
				{ "POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "NORMAL", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 12, D3D11_INPUT_PER_VERTEX_DATA, 0 },
			};
			a_device->CreateInputLayout(cubeElements, 2, cubeVsBlob->GetBufferPointer(), cubeVsBlob->GetBufferSize(), &g_cubeLayout);
			// GmskyRndVertex: position, normal, u v, 4 bone numbers, 4 weights (0-255 -> 0-1).
			const D3D11_INPUT_ELEMENT_DESC modelElements[] = {
				{ "POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, offsetof(GmskyRndVertex, pos), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "NORMAL", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, offsetof(GmskyRndVertex, normal), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0, offsetof(GmskyRndVertex, u), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "BLENDINDICES", 0, DXGI_FORMAT_R8G8B8A8_UINT, 0, offsetof(GmskyRndVertex, bones), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "BLENDWEIGHT", 0, DXGI_FORMAT_R8G8B8A8_UNORM, 0, offsetof(GmskyRndVertex, weights), D3D11_INPUT_PER_VERTEX_DATA, 0 },
			};
			a_device->CreateInputLayout(modelElements, 5, modelVsBlob->GetBufferPointer(), modelVsBlob->GetBufferSize(), &g_modelLayout);
			Release(cubeVsBlob);
			Release(cubePsBlob);
			Release(modelVsBlob);
			Release(modelPsBlob);

			D3D11_BUFFER_DESC cbd{};
			cbd.ByteWidth = sizeof(FrameConstants);
			cbd.Usage = D3D11_USAGE_DYNAMIC;
			cbd.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
			cbd.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
			a_device->CreateBuffer(&cbd, nullptr, &g_frameCb);
			cbd.ByteWidth = sizeof(ObjectConstants);
			a_device->CreateBuffer(&cbd, nullptr, &g_objectCb);

			// The cube: 6 faces, 2 triangles each, centred on 0.
			std::vector<CubeVertex> verts;
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
			vbd.ByteWidth = UINT(verts.size() * sizeof(CubeVertex));
			vbd.Usage = D3D11_USAGE_IMMUTABLE;
			vbd.BindFlags = D3D11_BIND_VERTEX_BUFFER;
			D3D11_SUBRESOURCE_DATA init{ verts.data(), 0, 0 };
			a_device->CreateBuffer(&vbd, &init, &g_cubeVb);

			D3D11_RASTERIZER_DESC rd{};
			rd.FillMode = D3D11_FILL_SOLID;
			rd.CullMode = D3D11_CULL_NONE;  // winding not checked yet (G5)
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

			D3D11_SAMPLER_DESC smp{};
			smp.Filter = D3D11_FILTER_ANISOTROPIC;
			smp.MaxAnisotropy = 8;
			smp.AddressU = smp.AddressV = smp.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
			smp.MaxLOD = D3D11_FLOAT32_MAX;
			a_device->CreateSamplerState(&smp, &g_sampler);

			// A 1x1 white texture for parts without one.
			const uint32_t white = 0xFFFFFFFFu;
			D3D11_TEXTURE2D_DESC wd{};
			wd.Width = wd.Height = 1;
			wd.MipLevels = wd.ArraySize = 1;
			wd.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
			wd.SampleDesc.Count = 1;
			wd.Usage = D3D11_USAGE_IMMUTABLE;
			wd.BindFlags = D3D11_BIND_SHADER_RESOURCE;
			D3D11_SUBRESOURCE_DATA wi{ &white, 4, 0 };
			a_device->CreateTexture2D(&wd, &wi, &g_whiteTex);
			if (g_whiteTex) {
				a_device->CreateShaderResourceView(g_whiteTex, nullptr, &g_whiteSrv);
			}

			const bool ok = g_cubeVs && g_cubePs && g_cubeLayout && g_modelVs && g_modelPs && g_modelLayout && g_frameCb && g_objectCb && g_cubeVb && g_raster &&
			                g_depthWrite && g_depthWriteRev && g_solidBlend && g_sampler && g_whiteSrv;
			if (!ok) {
				logger::error("in-frame drawing: failed to create its graphics objects");
				g_initFailed = true;
				return false;
			}
			g_device = a_device;
			g_device->AddRef();
			logger::info("in-frame drawing: ready (test cube {}, model drawing on)", kTestCube ? "on" : "off");
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
			ID3D11Buffer* ib{};
			DXGI_FORMAT ibFormat{};
			UINT ibOffset{};
			ID3D11VertexShader* vs{};
			ID3D11PixelShader* ps{};
			ID3D11Buffer* vsCb[2]{};
			ID3D11Buffer* psCb[2]{};
			ID3D11SamplerState* sampler{};
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
				a_c->IAGetIndexBuffer(&ib, &ibFormat, &ibOffset);
				a_c->VSGetShader(&vs, nullptr, nullptr);
				a_c->PSGetShader(&ps, nullptr, nullptr);
				a_c->VSGetConstantBuffers(0, 2, vsCb);
				a_c->PSGetConstantBuffers(0, 2, psCb);
				a_c->PSGetSamplers(0, 1, &sampler);
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
				a_c->IASetIndexBuffer(ib, ibFormat, ibOffset);
				a_c->VSSetShader(vs, nullptr, 0);
				a_c->PSSetShader(ps, nullptr, 0);
				a_c->VSSetConstantBuffers(0, 2, vsCb);
				a_c->PSSetConstantBuffers(0, 2, psCb);
				a_c->PSSetSamplers(0, 1, &sampler);
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
				Release(ib);
				Release(vs);
				Release(ps);
				for (auto*& b : vsCb) {
					Release(b);
				}
				for (auto*& b : psCb) {
					Release(b);
				}
				Release(sampler);
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

		// ---- G3a: Garry's Mod's models and textures onto the graphics card ----

		void FreeModel(GpuModel& a_m)
		{
			Release(a_m.vb);
			Release(a_m.ib);
		}

		void FreeTexture(GpuTexture& a_t)
		{
			Release(a_t.srv);
			Release(a_t.tex);
		}

		bool UploadModel(const Render::Model& a_src, GpuModel& a_out)
		{
			D3D11_BUFFER_DESC bd{};
			bd.Usage = D3D11_USAGE_IMMUTABLE;
			bd.BindFlags = D3D11_BIND_VERTEX_BUFFER;
			bd.ByteWidth = UINT(a_src.vertices.size() * sizeof(GmskyRndVertex));
			D3D11_SUBRESOURCE_DATA vi{ a_src.vertices.data(), 0, 0 };
			bd.BindFlags = D3D11_BIND_VERTEX_BUFFER;
			if (a_src.vertices.empty() || a_src.indices.empty() || FAILED(g_device->CreateBuffer(&bd, &vi, &a_out.vb))) {
				return false;
			}
			bd.BindFlags = D3D11_BIND_INDEX_BUFFER;
			bd.ByteWidth = UINT(a_src.indices.size() * 4);
			D3D11_SUBRESOURCE_DATA ii{ a_src.indices.data(), 0, 0 };
			if (FAILED(g_device->CreateBuffer(&bd, &ii, &a_out.ib))) {
				Release(a_out.vb);
				return false;
			}
			a_out.parts = a_src.parts;
			a_out.bones = a_src.header.bone_count;
			a_out.triangles = UINT(a_src.indices.size() / 3);
			return true;
		}

		bool UploadTexture(ID3D11DeviceContext* a_context, const Render::Texture& a_src, GpuTexture& a_out)
		{
			const auto& h = a_src.header;
			D3D11_TEXTURE2D_DESC td{};
			td.Width = h.width;
			td.Height = h.height;
			td.ArraySize = 1;
			td.SampleDesc.Count = 1;
			td.Usage = D3D11_USAGE_DEFAULT;
			td.BindFlags = D3D11_BIND_SHADER_RESOURCE;
			if (h.format == GMSKY_RND_TEX_RGBA8) {
				// Source's colour textures are sRGB; Skyrim makes the smaller mip levels.
				td.MipLevels = 0;
				td.Format = DXGI_FORMAT_R8G8B8A8_UNORM_SRGB;
				td.BindFlags |= D3D11_BIND_RENDER_TARGET;
				td.MiscFlags = D3D11_RESOURCE_MISC_GENERATE_MIPS;
				if (FAILED(g_device->CreateTexture2D(&td, nullptr, &a_out.tex)) || FAILED(g_device->CreateShaderResourceView(a_out.tex, nullptr, &a_out.srv))) {
					FreeTexture(a_out);
					return false;
				}
				a_context->UpdateSubresource(a_out.tex, 0, nullptr, a_src.pixels.data(), h.width * 4, 0);
				a_context->GenerateMips(a_out.srv);
				return true;
			}
			// BC1-3: one level (Direct3D 11 needs the top level a multiple of 4 wide and high).
			if (h.width % 4 || h.height % 4) {
				return false;
			}
			td.MipLevels = 1;
			td.Format = h.format == GMSKY_RND_TEX_BC1 ? DXGI_FORMAT_BC1_UNORM_SRGB : h.format == GMSKY_RND_TEX_BC2 ? DXGI_FORMAT_BC2_UNORM_SRGB : DXGI_FORMAT_BC3_UNORM_SRGB;
			td.Usage = D3D11_USAGE_IMMUTABLE;
			const UINT blockBytes = h.format == GMSKY_RND_TEX_BC1 ? 8 : 16;
			D3D11_SUBRESOURCE_DATA init{ a_src.pixels.data(), (h.width / 4) * blockBytes, 0 };
			if (FAILED(g_device->CreateTexture2D(&td, &init, &a_out.tex)) || FAILED(g_device->CreateShaderResourceView(a_out.tex, nullptr, &a_out.srv))) {
				FreeTexture(a_out);
				return false;
			}
			return true;
		}

		// What the render block holds now, onto the graphics card: new or changed models and
		// textures uploaded, ones Garry's Mod dropped (RESET, FORGET) freed.
		void SyncGpu(ID3D11DeviceContext* a_context, Seen& a_seen)
		{
			const auto start = Qpc();
			const auto& models = Render::Models();
			for (auto it = g_gpuModels.begin(); it != g_gpuModels.end();) {
				const auto src = models.find(it->first);
				if (src == models.end() || src->second.serial != it->second.serial) {
					FreeModel(it->second);
					it = g_gpuModels.erase(it);
				} else {
					++it;
				}
			}
			for (const auto& [id, src] : models) {
				if (g_gpuModels.contains(id)) {
					continue;
				}
				GpuModel m;
				m.serial = src.serial;
				if (UploadModel(src, m)) {
					++a_seen.modelUploads;
					logger::info("in-frame drawing: model {} uploaded: {} vertices, {} triangles, {} parts, {} bones", id, src.vertices.size(), m.triangles, m.parts.size(), m.bones);
				} else {
					++a_seen.uploadFailures;
					logger::warn("in-frame drawing: model {} could not be uploaded ({} vertices, {} indices)", id, src.vertices.size(), src.indices.size());
				}
				g_gpuModels[id] = std::move(m);  // kept even if it failed, so it isn't retried every frame
			}
			const auto& textures = Render::Textures();
			for (auto it = g_gpuTextures.begin(); it != g_gpuTextures.end();) {
				const auto src = textures.find(it->first);
				if (src == textures.end() || src->second.serial != it->second.serial) {
					FreeTexture(it->second);
					it = g_gpuTextures.erase(it);
				} else {
					++it;
				}
			}
			for (const auto& [id, src] : textures) {
				if (g_gpuTextures.contains(id)) {
					continue;
				}
				GpuTexture t;
				t.serial = src.serial;
				if (UploadTexture(a_context, src, t)) {
					++a_seen.textureUploads;
					logger::info("in-frame drawing: texture {} uploaded: {}x{} format {}", id, src.header.width, src.header.height, src.header.format);
				} else {
					++a_seen.uploadFailures;
					logger::warn("in-frame drawing: texture {} could not be uploaded ({}x{} format {})", id, src.header.width, src.header.height, src.header.format);
				}
				g_gpuTextures[id] = t;
			}
			a_seen.uploadMsMax = std::max(a_seen.uploadMsMax, QpcMs(Qpc() - start));
			a_seen.gpuModels = g_gpuModels.size();
			a_seen.gpuTextures = g_gpuTextures.size();
		}

		// The newest frame's matrices into the buffer the vertex shader reads.
		bool UploadBones(ID3D11DeviceContext* a_context, const Render::Frame& a_frame)
		{
			const UINT rows = UINT(a_frame.matrices.size() * 3);
			if (rows > g_boneCapacity) {
				Release(g_boneSrv);
				Release(g_boneBuf);
				g_boneCapacity = 0;
				const UINT cap = std::max<UINT>(rows * 2, 3 * 1024);
				D3D11_BUFFER_DESC bd{};
				bd.ByteWidth = cap * 16;
				bd.Usage = D3D11_USAGE_DYNAMIC;
				bd.BindFlags = D3D11_BIND_SHADER_RESOURCE;
				bd.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
				bd.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_STRUCTURED;
				bd.StructureByteStride = 16;
				if (FAILED(g_device->CreateBuffer(&bd, nullptr, &g_boneBuf)) || FAILED(g_device->CreateShaderResourceView(g_boneBuf, nullptr, &g_boneSrv))) {
					Release(g_boneBuf);
					return false;
				}
				g_boneCapacity = cap;
			}
			D3D11_MAPPED_SUBRESOURCE mapped{};
			if (FAILED(a_context->Map(g_boneBuf, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
				return false;
			}
			std::memcpy(mapped.pData, a_frame.matrices.data(), a_frame.matrices.size() * sizeof(GmskyRndMatrix));
			a_context->Unmap(g_boneBuf, 0);
			return true;
		}

		void DrawModels(ID3D11DeviceContext* a_context, const RE::NiPoint3& a_cam, const FrameConstants& a_fc, Seen& a_seen)
		{
			const auto& frame = Render::NewestFrame();
			a_seen.gmodFrame = frame.valid ? frame.header.frame : 0;
			if (!frame.valid || frame.objects.empty()) {
				return;
			}
			GmskyOrigin origin{};
			if (!Link::Get().IsOpen() || !SlotRead(Link::Get().Shared().origin, origin) || origin.epoch == 0) {
				a_seen.skippedNoOrigin += frame.objects.size();
				return;
			}
			a_seen.frameEpoch = frame.header.origin_epoch;
			a_seen.originEpoch = origin.epoch;
			if (frame.header.origin_epoch != origin.epoch) {
				a_seen.skippedEpoch += frame.objects.size();
				return;
			}
			if (!UploadBones(a_context, frame)) {
				return;
			}
			// The anchor (= Skyrim's origin) relative to the camera, in double precision.
			const float offset[3] = { float(origin.skyrim_origin[0] - double(a_cam.x)), float(origin.skyrim_origin[1] - double(a_cam.y)),
				float(origin.skyrim_origin[2] - double(a_cam.z)) };
			const float perGmod = float(1.0 / kGmodPerSkyrim);
			a_context->IASetInputLayout(g_modelLayout);
			a_context->VSSetShader(g_modelVs, nullptr, 0);
			a_context->PSSetShader(g_modelPs, nullptr, 0);
			ID3D11Buffer* cbs[2] = { g_frameCb, g_objectCb };
			a_context->VSSetConstantBuffers(0, 2, cbs);
			a_context->PSSetConstantBuffers(0, 2, cbs);
			a_context->VSSetShaderResources(0, 1, &g_boneSrv);
			a_context->PSSetSamplers(0, 1, &g_sampler);
			bool counted = false;
			for (const auto& o : frame.objects) {
				const auto it = g_gpuModels.find(o.model_id);
				if (it == g_gpuModels.end() || !it->second.vb) {
					++a_seen.skippedUnknown;
					continue;
				}
				const auto& m = it->second;
				if (o.matrix_count != m.bones) {
					++a_seen.skippedBones;
					continue;
				}
				const UINT stride = sizeof(GmskyRndVertex), zero = 0;
				a_context->IASetVertexBuffers(0, 1, &m.vb, &stride, &zero);
				a_context->IASetIndexBuffer(m.ib, DXGI_FORMAT_R32_UINT, 0);
				for (const auto& part : m.parts) {
					ObjectConstants oc{};
					oc.offset[0] = offset[0], oc.offset[1] = offset[1], oc.offset[2] = offset[2], oc.offset[3] = perGmod;
					oc.info[0] = o.first_matrix;
					oc.info[1] = part.flags;
					for (int c = 0; c < 4; ++c) {
						oc.color[c] = float((o.color >> (8 * c)) & 0xFF) / 255.0f;
					}
					oc.alpha[0] = part.alpha_ref;
					D3D11_MAPPED_SUBRESOURCE mapped{};
					if (FAILED(a_context->Map(g_objectCb, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
						continue;
					}
					std::memcpy(mapped.pData, &oc, sizeof(oc));
					a_context->Unmap(g_objectCb, 0);
					const auto tex = g_gpuTextures.find(part.texture_id);
					ID3D11ShaderResourceView* srv = tex != g_gpuTextures.end() && tex->second.srv ? tex->second.srv : g_whiteSrv;
					a_context->PSSetShaderResources(1, 1, &srv);
					a_context->DrawIndexed(part.index_count, part.first_index, 0);
					++a_seen.drawCalls;
					a_seen.triangles += part.index_count / 3;
				}
				++a_seen.objectsDrawn;
				if (!counted) {
					// Where the first object is in Skyrim's world and on screen (for the log).
					counted = true;
					const auto& mt = frame.matrices[o.first_matrix].m;
					const double world[3] = { origin.skyrim_origin[0] + mt[0][3] / kGmodPerSkyrim, origin.skyrim_origin[1] + mt[1][3] / kGmodPerSkyrim,
						origin.skyrim_origin[2] + mt[2][3] / kGmodPerSkyrim };
					const double rel[3] = { world[0] - a_cam.x, world[1] - a_cam.y, world[2] - a_cam.z };
					float clip[4];
					for (int r = 0; r < 4; ++r) {
						clip[r] = float(a_fc.viewProj[r][0] * rel[0] + a_fc.viewProj[r][1] * rel[1] + a_fc.viewProj[r][2] * rel[2] + a_fc.viewProj[r][3]);
					}
					a_seen.firstValid = true;
					std::copy(std::begin(world), std::end(world), a_seen.first);
					a_seen.firstDistance = float(std::sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]));
					a_seen.firstOnScreen = clip[3] > 1e-3f && std::fabs(clip[0] / clip[3]) <= 1.0f && std::fabs(clip[1] / clip[3]) <= 1.0f;
					a_seen.firstX = clip[3] > 1e-3f ? (clip[0] / clip[3] + 1) * 0.5f : -1.0f;
					a_seen.firstY = clip[3] > 1e-3f ? (1 - clip[1] / clip[3]) * 0.5f : -1.0f;
				}
			}
			++a_seen.framesDrawn;
		}

		void DrawInFrame()
		{
			g_calls.fetch_add(1, std::memory_order_relaxed);
			// G2: read what Garry's Mod sent (every frame, menus or not, so the ring never fills).
			Render::Drain();
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
			if (kTestCube ? (g_placeCube.exchange(false) || !g_cubePlaced) : false) {
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

			Seen seen;
			{
				std::lock_guard lock(g_seenLock);
				seen = g_seen;
			}
			seen.reversed = reversed;
			seen.depthFrom = probeKnown ? "probe" : "matrix";
			if constexpr (kTestCube) {
				float clip[4];
				for (int r = 0; r < 4; ++r) {
					clip[r] = fc.viewProj[r][0] * fc.offset[0] + fc.viewProj[r][1] * fc.offset[1] + fc.viewProj[r][2] * fc.offset[2] + fc.viewProj[r][3];
				}
				seen.cubeValid = true;
				seen.cam[0] = cam.x, seen.cam[1] = cam.y, seen.cam[2] = cam.z;
				std::copy(std::begin(g_cube), std::end(g_cube), seen.cube);
				seen.distance = float(std::sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]));
				seen.onScreen = clip[3] > 1e-3f && std::fabs(clip[0] / clip[3]) <= 1.0f && std::fabs(clip[1] / clip[3]) <= 1.0f;
				seen.screenX = clip[3] > 1e-3f ? (clip[0] / clip[3] + 1) * 0.5f : -1.0f;
				seen.screenY = clip[3] > 1e-3f ? (1 - clip[1] / clip[3]) * 0.5f : -1.0f;
			}

			StateBackup backup;
			backup.Save(context);
			SyncGpu(context, seen);
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
			context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
			if constexpr (kTestCube) {
				const UINT stride = sizeof(CubeVertex), zero = 0;
				context->IASetInputLayout(g_cubeLayout);
				context->IASetVertexBuffers(0, 1, &g_cubeVb, &stride, &zero);
				context->VSSetShader(g_cubeVs, nullptr, 0);
				context->PSSetShader(g_cubePs, nullptr, 0);
				context->VSSetConstantBuffers(0, 1, &g_frameCb);
				context->PSSetConstantBuffers(0, 1, &g_frameCb);
				context->Draw(36, 0);
			}
			DrawModels(context, cam, fc, seen);
			backup.Restore(context);

			{
				std::lock_guard lock(g_seenLock);
				g_seen = seen;
			}
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
			// Per-second counts start again; what is held / last seen stays.
			g_seen.objectsDrawn = g_seen.skippedEpoch = g_seen.skippedNoOrigin = g_seen.skippedUnknown = g_seen.skippedBones = 0;
			g_seen.triangles = g_seen.drawCalls = g_seen.framesDrawn = 0;
			g_seen.modelUploads = g_seen.textureUploads = g_seen.uploadFailures = 0;
			g_seen.uploadMsMax = 0;
		}
		std::string cube;
		if (seen.cubeValid) {
			cube = std::format("; cube at ({:.1f}, {:.1f}, {:.1f}), camera ({:.1f}, {:.1f}, {:.1f}), {:.0f} units away, {}", seen.cube[0], seen.cube[1], seen.cube[2],
				seen.cam[0], seen.cam[1], seen.cam[2], seen.distance,
				seen.onScreen ? std::format("on screen at ({:.3f}, {:.3f})", seen.screenX, seen.screenY) : std::string("off screen"));
		}
		std::string first;
		if (seen.firstValid) {
			first = std::format("; first object at ({:.1f}, {:.1f}, {:.1f}), {:.0f} units away, {}", seen.first[0], seen.first[1], seen.first[2], seen.firstDistance,
				seen.firstOnScreen ? std::format("on screen at ({:.3f}, {:.3f})", seen.firstX, seen.firstY) : std::string("off screen"));
		}
		const uint64_t objectsDrawn = seen.objectsDrawn;
		logger::info("in-frame drawing: {} calls, {} drawn, skipped: {} menu, {} no camera/player, {} targets, {} init; {:.3f} ms avg, {:.3f} max (CPU); depth {} ({}){}",
			calls, drawn, menu, noCam, targets, init, drawn ? QpcMs(ticks) / drawn : 0.0, QpcMs(maxTicks), seen.reversed ? "reversed" : "standard", seen.depthFrom, cube);
		if (seen.gpuModels || seen.gmodFrame || seen.uploadFailures) {
			logger::info("in-frame drawing: Garry's Mod objects: {} frames drawn, {} objects ({} triangles, {} draw calls); skipped: {} epoch (frame {} vs origin {}), {} no origin, {} unknown model, {} bone count; "
						 "on the card: {} models, {} textures; uploads {} models, {} textures, {} failed, max {:.2f} ms; Garry's Mod frame {}{}",
				seen.framesDrawn, objectsDrawn, seen.triangles, seen.drawCalls, seen.skippedEpoch, seen.frameEpoch, seen.originEpoch, seen.skippedNoOrigin, seen.skippedUnknown,
				seen.skippedBones, seen.gpuModels, seen.gpuTextures, seen.modelUploads, seen.textureUploads, seen.uploadFailures, seen.uploadMsMax, seen.gmodFrame, first);
		}
	}
}
