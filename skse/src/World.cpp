// Geometry route (DESIGN.md §12, "Garry's Mod's things drawn inside Skyrim"): Skyrim draws Garry's
// Mod's objects itself, inside its own frame. Adapted from SkyCraft (MIT), skse/src/WorldRender.cpp:
// a hook right after the call to Main::RenderWorld (REL::ID(36559) + 0x85E, checked to be a call to
// REL::ID(107142) before patching). There Skyrim's 3D scene is complete and its post-processing
// (tone mapping, bloom, TAA) hasn't run, so what we draw into its HDR scene target with its main
// depth buffer bound is depth-tested against Skyrim's world, writes Skyrim's depth, and goes
// through Skyrim's post-processing like its own objects. Motion vectors and normals are written
// too, so Skyrim's TAA and ambient occlusion treat our surfaces like its own. Camera: Skyrim's own
// world -> clip matrix of this frame, re-based on the camera position (SkyCraft's RenderPasses);
// every graphics setting is saved and put back (SkyCraft's StateBackup).
//
// G1: a fixed test cube (kTestCube; off since G3a). G3a: the models, textures and newest frame
// from the render block (Render.cpp) are uploaded and drawn. Each model is bent on the graphics
// card by its object's bone matrices (up to 4 bones per corner; what Source itself does) and
// placed at origin - camera + position / scale (§5; the subtraction in double precision here).
// Differs from SkyCraft: SkyCraft receives posed triangles every frame; we receive each model once
// and its bone matrices every frame.
//
// G5a: Skyrim's light (SkyCraft's GatherLighting / Lighting, HDR path): its directional light,
// directional ambient, the 16 point lights nearest to reaching the camera, and fog. See-through
// and glowing parts ($translucent / $additive) in a second, blended pass without depth writes.
// Shadows are G5b. G6: an object the player holds is kept where it was in front of Garry's Mod's
// camera, in front of Skyrim's camera of this frame (SkyCraft's rule for the player's body, sent
// relative to the feet the camera follows); every object's motion vectors use its own matrices of
// the frame before, so TAA sees objects move.
//
// G5b bundle: shadows (SkyCraft's, MIT). Skyrim's sun cascades are copied the moment Skyrim has
// drawn them (a hook on BSShadowDirectionalLight's Render, vtable entry 0x0A) and read by our pixel
// shader (SkyrimSunShadow). Our objects go into our own sun shadow map (SetSunShadowMatrix), used
// by our pixel shader (they shadow themselves and each other) and by a full-screen multiply pass
// before them (OverlayPS) that takes the sun's share of light from Skyrim's pixels our objects
// block; Skyrim's own shadow mask, once a probe shows it is usable, avoids double shadows. Normal
// maps ($bumpmap, tangent per corner) and phong shine ($phong, exponent, boost, mask) in the object
// shader. Garry's Mod's console switches (frame flags) turn each off live.
//
// Phong as Source's VertexLitGeneric does it (after test 84, "drenched in sweat"): Fresnel ranges,
// $phongtint, $phongexponenttexture / $phongalbedotint, highlight x n.l. Indoor shadow probe (I1):
// Skyrim's frustum and parabolic shadow lights are hooked (Render, vtable entry 0x0A) and logged
// only, to learn their shadow maps before drawing with them.
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
		constexpr int kMaxLights = 16;           // SkyCraft's number
		constexpr int kMaxBlobs = 24;            // contact shadows (SkyCraft's number)
		constexpr int kProbeCharacters = 5;      // step N1: characters whose sun shadow is probed (3 points each)
		constexpr int kProbePoints = 16;
		constexpr float kPi = 3.14159265f;
		// Our sun shadow map (SkyCraft's numbers): objects within kShadowRadius (sideways from the
		// sun) of the camera, kShadowSize texels across, kShadowDepth either way along the sun.
		constexpr UINT kShadowSize = 2048;
		constexpr float kShadowRadius = 64.0f * 70.0f;
		constexpr float kShadowDepth = 12000.0f;

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
#define MAX_LIGHTS 16
cbuffer Frame : register(b0)
{
	row_major float4x4 viewProj;      // camera-relative Skyrim units -> clip (this frame)
	row_major float4x4 prevViewProj;  // last frame's camera, re-based on this frame's (motion vectors)
	float4 offset;                    // the test cube's centre, camera-relative
	float4 lightDir;                  // the test cube's fixed light
	float4 sunDir;                    // towards the sun, moon or interior light (Skyrim axes)
	float4 sunColor;                  // rgb; w: point light count
	float4 ambient[6];                // Skyrim's directional ambient: faces looking +X, -X, +Y, -Y, +Z, -Z
	float4 fogRange;                  // 1/(far-near), near/(far-near), power, max amount
	float4 fogNear;                   // rgb; w: fog on
	float4 fogFar;                    // rgb
	float4 camRight;                  // camera axes (Skyrim world), for Skyrim's view-space normals
	float4 camUp;
	float4 camForward;
	float4 lightPos[MAX_LIGHTS];      // camera-relative; w: 1 / radius
	float4 lightColor[MAX_LIGHTS];
	row_major float4x4 lightViewProj;     // camera-relative Skyrim units -> our sun shadow map
	row_major float4x4 invViewProj;       // screen (ndc x, y, Skyrim depth) -> camera-relative Skyrim units
	float4 shadowParams;                  // x: sun shadows on, y: normal offset (units), z: shadow map texel (uv), w: depth bias
	float4 shadowExtra;                   // x: Skyrim's shadow mask usable, y: squared reach of our shadow map (units)
	row_major float4x4 skyShadowProj[2];  // Skyrim's sun shadow cascades: camera-relative -> shadow map uv, depth
	float4 skyShadowSplits;               // cascade 0 end, cascade 1 end (view depth), -, cascade count
	float4 skyShadowParams;               // slice of cascade 0, slice of cascade 1, texel size (uv), +1 standard depth / -1 reversed
	float4 depthParams;                   // near, far, reversed
	float4 screen;                        // 1 / width, 1 / height
	float4 switches;                      // x: normal maps' green the other way, y: normal maps on, z: phong on, w: colour textures to linear
	float4 aoParams;                      // x: contact shadow count; y: shadow debug colours (gmsky_render_shadow_debug 1)
	float4 aoBlobs[24];                   // Garry's Mod's NPCs' and players' feet, camera-relative; w: radius (step N1)
	float4 probePts[16];                  // the NPC shadow probe's points, camera-relative
};
cbuffer Object : register(b1)
{
	float4 objOffset;  // xyz: the anchor (Skyrim's origin) relative to the camera, Skyrim units; w: Skyrim units per Garry's Mod unit
	uint4  objInfo;    // x: the object's first matrix in boneRows (x 3 rows); y: GMSKY_RND_MAT_* of this part; z: held; w: has a normal map
	float4 objColor;   // the object's tint
	float4 objAlpha;   // x: alpha reference (alpha-tested parts); y: phong exponent; z: phong boost; w: a character (has a contact shadow)
	float4 held[3];    // held objects: camera-relative position then -> now (rows of [rotation | translation])
	float4 objFresnel; // xyz: $phongfresnelranges (head-on, middle, grazing); w: $phongalbedotint
	float4 objTint;    // rgb: $phongtint; w: has a $phongexponenttexture
	float4 objSelfIllum;  // rgb: $selfillumtint; w: has a $selfillummask
};
StructuredBuffer<float4> boneRows : register(t0);      // every matrix of the frame, 3 rows each
Texture2D albedo : register(t1);
StructuredBuffer<float4> prevBoneRows : register(t2);  // the same objects' matrices on the frame before
Texture2D normalMap : register(t3);                    // the part's $bumpmap (linear)
Texture2D<float> shadowMap : register(t4);             // our objects seen from the sun
Texture2DArray<float> skyrimShadowMaps : register(t5); // Skyrim's sun cascades, copied
Texture2D<float> sceneDepth : register(t6);            // Skyrim's depth (the shadow pass on Skyrim)
Texture2D skyrimMask : register(t7);                   // Skyrim's screen-space shadow mask (r: sun, 1 = lit)
Texture2D phongExponent : register(t8);                // the part's $phongexponenttexture (linear)
Texture2D selfIllumMask : register(t9);                // the part's $selfillummask (linear)
SamplerState albedoSampler : register(s0);
SamplerState pointSampler : register(s1);
SamplerComparisonState shadowSampler : register(s2);

struct PSOut
{
	float4 color : SV_Target0;
	float4 motion : SV_Target1;  // Skyrim's motion vectors (for its TAA), when bound
	float4 normal : SV_Target2;  // Skyrim's normals + TAA / reflection masks, when bound
};

float2 Motion(float4 cur, float4 prev)
{
	return float2(-0.5, 0.5) * (cur.xy / cur.w - prev.xy / prev.w);  // Skyrim's convention: previous uv - current uv
}

// Skyrim's normal buffer encoding (view space: x right, y up, z forward): nothing special for
// TAA, no screen-space reflections (SkyCraft's EncodeSkyrimNormal).
float4 EncodeSkyrimNormal(float3 n)
{
	float3 v = float3(dot(n, camRight.xyz), dot(n, camUp.xyz), dot(n, camForward.xyz));
	float  f = max(0.001, sqrt(8.0 - 8.0 * v.z));
	return float4(v.xy / f + 0.5, 0.0, 0.0);
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
	po.normal = EncodeSkyrimNormal(normalize(i.normal));
	return po;
}

// ---- Garry's Mod's models ----
struct ModelIn
{
	float3 pos : POSITION;
	float3 normal : NORMAL;
	float4 tangent : TANGENT;
	float2 uv : TEXCOORD0;
	uint4  bones : BLENDINDICES;
	float4 weights : BLENDWEIGHT;
};
struct ModelOut
{
	float4 pos : SV_Position;
	float3 normal : NORMAL;
	float3 tangent : TANGENT;
	float2 uv : TEXCOORD0;
	float4 curClip : TEXCOORD1;
	float4 prevClip : TEXCOORD2;
	float3 rel : TEXCOORD3;  // camera-relative Skyrim position
	float  viewZ : TEXCOORD4;
	nointerpolation float handed : TEXCOORD5;
};

float3 Held(float3 p)
{
	return float3(dot(held[0], float4(p, 1.0)), dot(held[1], float4(p, 1.0)), dot(held[2], float4(p, 1.0)));
}

float3 HeldDir(float3 d)
{
	return float3(dot(held[0].xyz, d), dot(held[1].xyz, d), dot(held[2].xyz, d));
}

// Bends a corner by its bones: position now (p), on the frame before (q), normal and tangent now.
void Skin(ModelIn i, out float3 p, out float3 q, out float3 n, out float3 t)
{
	p = 0, q = 0, n = 0, t = 0;
	float4 hp = float4(i.pos, 1.0);
	[unroll] for (int k = 0; k < 4; ++k) {
		float w = i.weights[k];
		if (w > 0.0) {
			uint   b = (objInfo.x + i.bones[k]) * 3;
			float4 r0 = boneRows[b], r1 = boneRows[b + 1], r2 = boneRows[b + 2];
			p += w * float3(dot(r0, hp), dot(r1, hp), dot(r2, hp));
			n += w * float3(dot(r0.xyz, i.normal), dot(r1.xyz, i.normal), dot(r2.xyz, i.normal));
			t += w * float3(dot(r0.xyz, i.tangent.xyz), dot(r1.xyz, i.tangent.xyz), dot(r2.xyz, i.tangent.xyz));
			float4 s0 = prevBoneRows[b], s1 = prevBoneRows[b + 1], s2 = prevBoneRows[b + 2];
			q += w * float3(dot(s0, hp), dot(s1, hp), dot(s2, hp));
		}
	}
}

ModelOut ModelVS(ModelIn i)
{
	float3 p, q, n, t;
	Skin(i, p, q, n, t);
	ModelOut o;
	float3   rel = objOffset.xyz + p * objOffset.w;  // Garry's Mod and Skyrim share their axes (Z up)
	float3   prevRel = objOffset.xyz + q * objOffset.w;
	if (objInfo.z != 0) {
		// Held: the same place in front of Skyrim's camera as it had in front of Garry's Mod's.
		rel = Held(rel);
		n = HeldDir(n);
		t = HeldDir(t);
	}
	o.pos = mul(viewProj, float4(rel, 1.0));
	o.normal = n;
	o.tangent = t;
	o.handed = i.tangent.w;
	o.uv = i.uv;
	o.rel = rel;
	o.viewZ = o.pos.w;
	o.curClip = o.pos;
	o.prevClip = objInfo.z != 0 ? o.pos : mul(prevViewProj, float4(prevRel, 1.0));  // held: no motion on screen
	return o;
}

// Our objects into our sun shadow map (depth only; alpha-tested texels don't cast).
struct ShadowOut
{
	float4 pos : SV_Position;
	float2 uv : TEXCOORD0;
};
ShadowOut ShadowVS(ModelIn i)
{
	float3 p, q, n, t;
	Skin(i, p, q, n, t);
	float3 rel = objOffset.xyz + p * objOffset.w;
	if (objInfo.z != 0) {
		rel = Held(rel);
	}
	ShadowOut o;
	o.pos = mul(lightViewProj, float4(rel, 1.0));
	o.uv = i.uv;
	return o;
}
void ShadowPS(ShadowOut i)
{
	if ((objInfo.y & 0x4u) && albedo.SampleLevel(albedoSampler, i.uv, 0).a * objColor.a < objAlpha.x) {
		discard;
	}
}

// Ambient from Skyrim's six directional colours, as its own lighting shader applies them.
float3 Ambient(float3 n)
{
	float3 n2 = n * n;
	return n2.x * (n.x >= 0 ? ambient[0].rgb : ambient[1].rgb)
	     + n2.y * (n.y >= 0 ? ambient[2].rgb : ambient[3].rgb)
	     + n2.z * (n.z >= 0 ? ambient[4].rgb : ambient[5].rgb);
}

// How much sun reaches a camera-relative point past our objects: 1 lit, 0 shadowed (SkyCraft's
// SunShadow). n pushes the lookup off the surface so faces don't shadow themselves.
float SunShadow(float3 rel, float3 n)
{
	if (shadowParams.x < 0.5) {
		return 1.0;
	}
	float4 lp = mul(lightViewProj, float4(rel + n * shadowParams.y, 1.0));
	float  edge = max(abs(lp.x), abs(lp.y));
	if (edge >= 1.0 || lp.z >= 1.0) {
		return 1.0;
	}
	float2 uv = float2(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
	float  ref = lp.z - shadowParams.w;
	float  lit = 0.0;
	[unroll] for (int y = -1; y <= 1; ++y) {
		[unroll] for (int x = -1; x <= 1; ++x) {
			lit += shadowMap.SampleCmpLevelZero(shadowSampler, uv + float2(x, y) * shadowParams.z, ref);
		}
	}
	return lerp(1.0, lit / 9.0, saturate((1.0 - edge) * 8.0));  // fades out towards the map's edge
}

// How much sun reaches a camera-relative point past Skyrim's own geometry, from the shadow maps
// Skyrim rendered for the sun this frame (SkyCraft's SkyrimSunShadow): 1 lit, 0 shadowed.
float SkyrimSunShadow(float3 rel, float3 n, float viewZ)
{
	if (skyShadowSplits.w < 0.5) {
		return 1.0;
	}
	uint cascade = viewZ < skyShadowSplits.x ? 0 : 1;
	if (cascade >= (uint)skyShadowSplits.w || viewZ >= (cascade == 0 ? skyShadowSplits.x : skyShadowSplits.y)) {
		return 1.0;  // beyond Skyrim's shadow range
	}
	float4 ls = mul(skyShadowProj[cascade], float4(rel + n * 6.0, 1.0));
	ls.xyz /= ls.w;
	if (any(ls.xy <= 0.0) || any(ls.xy >= 1.0)) {
		return 1.0;
	}
	float slice = cascade == 0 ? skyShadowParams.x : skyShadowParams.y;
	float lit = 0.0;
	[unroll] for (int y = -1; y <= 1; ++y) {
		[unroll] for (int x = -1; x <= 1; ++x) {
			float d = skyrimShadowMaps.SampleLevel(pointSampler, float3(ls.xy + float2(x, y) * skyShadowParams.z, slice), 0);
			lit += skyShadowParams.w > 0.0 ? (ls.z - 0.0005 <= d ? 1.0 : 0.0) : (ls.z + 0.0005 >= d ? 1.0 : 0.0);
		}
	}
	float end = skyShadowSplits.w > 1.5 ? skyShadowSplits.y : skyShadowSplits.x;
	return lerp(lit / 9.0, 1.0, saturate((viewZ - end * 0.85) / (end * 0.15)));  // fade out at the far end
}

// Soft contact shadows under Garry's Mod's NPCs and players (SkyCraft's BlobAO, MIT: their bodies
// are drawn after Skyrim's own ambient occlusion, so they'd otherwise look pasted onto the ground):
// the ground darkens most right under the feet and fades out over about a body's width.
// Ground-facing surfaces only.
float BlobAO(float3 rel, float3 n)
{
	float up = saturate(n.z * 1.25);
	float ao = 1.0;
	if (up <= 0.0) {
		return ao;
	}
	for (int k = 0; k < (int)aoParams.x; ++k) {
		float3 dv = rel - aoBlobs[k].xyz;
		float  r = aoBlobs[k].w;
		float  h = length(dv.xy) / r;
		if (h < 1.0 && dv.z > -0.45 * r && dv.z < 0.4 * r) {
			float fall = 1.0 - h;
			float vert = 1.0 - saturate(abs(dv.z) / (0.45 * r));
			ao *= 1.0 - 0.6 * fall * fall * vert * up;
		}
	}
	return ao;
}

)"  // MSVC caps one string literal at 16 KB
			R"(PSOut ModelPS(ModelOut i)
{
	PSOut  po;
	float4 t = albedo.Sample(albedoSampler, i.uv);
	if (switches.w > 0.5) {
		t.rgb = pow(t.rgb, 2.2);  // the old way (before test 86): sRGB -> linear, darker than Skyrim's own surfaces
	}
	t *= objColor;
	uint   flags = objInfo.y;
	if (flags & 0x4u) {  // GMSKY_RND_MAT_ALPHATEST
		clip(t.a - objAlpha.x);
	}
	float3 geo = normalize(i.normal);
	float  side = dot(geo, i.rel) > 0 ? -1.0 : 1.0;  // a face seen from behind (no culling yet) is lit from our side
	geo *= side;
	float3 n = geo;
	float  mask = 1.0;
	float3 tan = i.tangent - geo * dot(geo, i.tangent);
	if (objInfo.w != 0 && switches.y > 0.5 && dot(tan, tan) > 1e-8) {
		// Normal map: tangent-space normal onto the surface (B = handedness x (N x T)).
		float4 nm = normalMap.Sample(albedoSampler, i.uv);
		float3 tn = nm.xyz * 2.0 - 1.0;
		tn.y = switches.x > 0.5 ? -tn.y : tn.y;
		float3 T = normalize(tan);
		float3 B = cross(geo, T) * i.handed * side;
		n = normalize(T * tn.x + B * tn.y + geo * max(tn.z, 0.05));
		mask = nm.a;
	}
	if (flags & 0x40u) {  // GMSKY_RND_MAT_BASEALPHA_PHONGMASK
		mask = t.a;
	}
	float3 c;
	float3 L = sunDir.xyz;
	float  sunVisible = shadowParams.x > 0.5 ? SunShadow(i.rel, geo) * SkyrimSunShadow(i.rel, geo, i.viewZ) : 1.0;
	{
		float3 lit = Ambient(n) + sunColor.rgb * (saturate(dot(n, L)) * sunVisible);
		float3 spec = 0;
		bool   phong = (flags & 0x20u) && switches.z > 0.5;
		float3 V = normalize(-i.rel);
		// Source's phong (VertexLitGeneric): exponent and albedo tint from the exponent texture when
		// there is one, Fresnel by the view angle, the highlight x n.l, then boost x mask x tint.
		float  expo = max(objAlpha.y, 1.0);
		float3 ptint = objTint.rgb;
		if (phong && objTint.w > 0.5) {
			float4 e = phongExponent.Sample(albedoSampler, i.uv);
			expo = 1.0 + 149.0 * e.r;
			if (objFresnel.w > 0.5) {
				ptint = lerp(float3(1, 1, 1), t.rgb, e.g);
			}
		} else if (phong && objFresnel.w > 0.5) {
			ptint = t.rgb;
		}
		float ff = 1.0 - saturate(dot(n, V));
		ff *= ff;
		float fresnel = ff < 0.5 ? lerp(objFresnel.x, objFresnel.y, 2.0 * ff) : lerp(objFresnel.y, objFresnel.z, 2.0 * ff - 1.0);
		if (phong && dot(geo, L) > 0.0) {
			spec += sunColor.rgb * sunVisible * pow(saturate(dot(reflect(-L, n), V)), expo) * saturate(dot(n, L));
		}
		uint count = (uint)sunColor.w;
		for (uint k = 0; k < count; ++k) {
			float3 d = lightPos[k].xyz - i.rel;
			float  f = saturate(length(d) * lightPos[k].w);
			float3 l = normalize(d);
			float3 power = lightColor[k].rgb * (1.0 - f * f);
			lit += power * saturate(dot(n, l));
			if (phong && dot(geo, l) > 0.0) {
				spec += power * pow(saturate(dot(reflect(-l, n), V)), expo) * saturate(dot(n, l));
			}
		}
		c = t.rgb * max(lit, 0.0);
		if (flags & 0x8u) {
			// Self-illuminated (Source's VertexLitGeneric $selfillum): the lit surface blends to its own
			// colour x $selfillumtint by the mask, so only the glowing bits ignore light and shadow
			// (before test 88 the whole part did).
			float3 glowMask = objSelfIllum.w > 0.5 ? selfIllumMask.Sample(albedoSampler, i.uv).rgb : t.aaa;
			c = lerp(c, t.rgb * objSelfIllum.rgb, glowMask);
		}
		c += spec * (objAlpha.z * mask * fresnel) * ptint;
		if (objAlpha.w < 0.5) {
			c *= BlobAO(i.rel, geo);  // a prop next to a character's feet (SkyCraft darkens its blocks so)
		}
		if (aoParams.y > 0.5) {
			// gmsky_render_shadow_debug 1 (step N1): red = in Skyrim's sun shadow, green = Skyrim's sun
			// reaches it, blue = in our objects' shadow; yellow = sun shadows off here.
			float sky = SkyrimSunShadow(i.rel, geo, i.viewZ);
			float ours = SunShadow(i.rel, geo);
			c = shadowParams.x > 0.5 ? float3(1.0 - sky, sky, 1.0 - ours) * 0.5 : float3(0.5, 0.5, 0.0);
		}
	}
	if (fogNear.w > 0.5) {
		float f = min(pow(saturate(length(i.rel) * fogRange.x - fogRange.y), fogRange.z), fogRange.w);
		c = (flags & 0x2u) ? c * (1.0 - f) : lerp(c, lerp(fogNear.rgb, fogFar.rgb, f), f);  // additive light fades out in fog
	}
	float a = (flags & 0x1u) ? t.a : 1.0;  // GMSKY_RND_MAT_TRANSLUCENT: blended by its alpha
	if (flags & 0x2u) {  // GMSKY_RND_MAT_ADDITIVE: added light
		c *= t.a;
		a = 1.0;
	}
	po.color = float4(c, a);
	po.motion = float4(Motion(i.curClip, i.prevClip), 0.0, 1.0);
	po.normal = EncodeSkyrimNormal(n);
	return po;
}

// ---- Full-screen passes ----
float4 FullVS(uint id : SV_VertexID) : SV_Position
{
	float2 uv = float2((id << 1) & 2, id & 2);
	return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

// Our objects' shadows on Skyrim (SkyCraft's OverlayPS, without its contact shadows): each pixel's
// position comes back from Skyrim's depth; where our objects hide it from the sun, the sun's share
// of its light is taken away. Drawn with a multiplying blend before our objects themselves.
float4 OverlayPS(float4 pos : SV_Position) : SV_Target
{
	float2 uv = pos.xy * screen.xy;
	float  d = sceneDepth.SampleLevel(pointSampler, uv, 0);
	bool   isSky = depthParams.z > 0.5 ? d <= 1e-6 : d >= 0.999999;
	if (isSky) {
		discard;
	}
	float4 h = mul(invViewProj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0));
	float3 rel = h.xyz / h.w;
	float3 n = normalize(cross(ddy(rel), ddx(rel)));
	n = dot(n, rel) > 0 ? -n : n;
	float shadow = dot(rel, rel) > shadowExtra.y ? 1.0 : SunShadow(rel, n);
	float ao = BlobAO(rel, n);  // under NPCs and players (step N1, SkyCraft's)
	if (shadow > 0.999 && ao > 0.999) {
		discard;
	}
	float  skyrimLit = shadowExtra.x > 0.5 ? saturate(skyrimMask.SampleLevel(pointSampler, uv, 0).r) : 1.0;
	float3 lumW = float3(0.2125, 0.7154, 0.0721);
	float  ambL = dot(Ambient(n), lumW);
	float  sunL = dot(sunColor.rgb, lumW) * saturate(dot(n, sunDir.xyz));
	float  keep = (ambL + 0.02) / (ambL + sunL + 0.02);
	return float4(lerp(1.0, keep, (1.0 - shadow) * skyrimLit).xxx * ao, 1.0);
}

// 16 samples of Skyrim's shadow mask across the screen (SkyCraft's probe points), to see whether it
// holds usable values before trusting it.
static const float2 kProbe[16] = {
	float2(0.1, 0.1), float2(0.3, 0.1), float2(0.5, 0.1), float2(0.7, 0.1), float2(0.9, 0.1),
	float2(0.2, 0.35), float2(0.5, 0.35), float2(0.8, 0.35),
	float2(0.2, 0.65), float2(0.5, 0.65), float2(0.8, 0.65),
	float2(0.1, 0.9), float2(0.3, 0.9), float2(0.5, 0.9), float2(0.7, 0.9), float2(0.9, 0.9) };
float MaskProbePS(float4 pos : SV_Position) : SV_Target
{
	return skyrimMask.SampleLevel(pointSampler, kProbe[(uint)pos.x], 0).r;
}

// Step N1: the sun shadow at the NPC probe's points, offset towards the sun, two pixels each:
// (Skyrim's, ours, cascade or -1 out of range, view depth), (Skyrim's map uv, depth, stored depth).
float4 ShadowProbePS(float4 pos : SV_Position) : SV_Target
{
	uint   px = (uint)pos.x;
	float3 rel = probePts[px / 2].xyz;
	float3 n = sunDir.xyz;
	float  viewZ = mul(viewProj, float4(rel, 1.0)).w;
	uint   cascade = viewZ < skyShadowSplits.x ? 0 : 1;
	bool   inRange = skyShadowSplits.w > 0.5 && cascade < (uint)skyShadowSplits.w && viewZ < (cascade == 0 ? skyShadowSplits.x : skyShadowSplits.y);
	if ((px & 1) == 0) {
		return float4(SkyrimSunShadow(rel, n, viewZ), SunShadow(rel, n), inRange ? (float)cascade : -1.0, viewZ);
	}
	float4 ls = mul(skyShadowProj[cascade], float4(rel + n * 6.0, 1.0));
	ls.xyz /= ls.w;
	float d = skyrimShadowMaps.SampleLevel(pointSampler, float3(saturate(ls.xy), cascade == 0 ? skyShadowParams.x : skyShadowParams.y), 0);
	return float4(ls.xy, ls.z, d);
}
)";

		struct FrameConstants
		{
			float viewProj[4][4];
			float prevViewProj[4][4];
			float offset[4];
			float lightDir[4];
			float sunDir[4];
			float sunColor[4];
			float ambient[6][4];
			float fogRange[4];
			float fogNear[4];
			float fogFar[4];
			float camRight[4];
			float camUp[4];
			float camForward[4];
			float lightPos[kMaxLights][4];
			float lightColor[kMaxLights][4];
			float lightViewProj[4][4];
			float invViewProj[4][4];
			float shadowParams[4];
			float shadowExtra[4];
			float skyShadowProj[2][4][4];
			float skyShadowSplits[4];
			float skyShadowParams[4];
			float depthParams[4];
			float screen[4];
			float switches[4];
			float aoParams[4];
			float aoBlobs[kMaxBlobs][4];
			float probePts[kProbePoints][4];
		};
		static_assert(sizeof(FrameConstants) % 16 == 0);

		struct ObjectConstants
		{
			float offset[4];
			uint32_t info[4];
			float color[4];
			float alpha[4];
			float held[3][4];
			float fresnel[4];
			float tint[4];
			float selfIllum[4];
		};
		static_assert(sizeof(ObjectConstants) == 160);

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
		ID3D11DepthStencilState* g_depthRead = nullptr;      // see-through parts: tested, not written
		ID3D11DepthStencilState* g_depthReadRev = nullptr;
		ID3D11BlendState* g_solidBlend = nullptr;
		ID3D11BlendState* g_alphaBlend = nullptr;  // $translucent
		ID3D11BlendState* g_addBlend = nullptr;    // $additive
		ID3D11SamplerState* g_sampler = nullptr;
		ID3D11Texture2D* g_whiteTex = nullptr;
		ID3D11ShaderResourceView* g_whiteSrv = nullptr;
		// The newest frame's matrices and the same objects' matrices on the frame before (rows as float4).
		struct RowBuffer
		{
			ID3D11Buffer* buf = nullptr;
			ID3D11ShaderResourceView* srv = nullptr;
			UINT capacity = 0;
		};
		RowBuffer g_bones, g_prevBones;
		// G5b: shadows.
		ID3D11VertexShader* g_shadowVs = nullptr;
		ID3D11PixelShader* g_shadowPs = nullptr;
		ID3D11VertexShader* g_fullVs = nullptr;
		ID3D11PixelShader* g_overlayPs = nullptr;
		ID3D11PixelShader* g_maskProbePs = nullptr;
		ID3D11SamplerState* g_pointSampler = nullptr;
		ID3D11SamplerState* g_shadowSampler = nullptr;
		ID3D11RasterizerState* g_shadowRaster = nullptr;
		ID3D11BlendState* g_multiplyBlend = nullptr;
		ID3D11BlendState* g_noBlend = nullptr;
		ID3D11Texture2D* g_shadowTex = nullptr;
		ID3D11DepthStencilView* g_shadowDsv = nullptr;
		ID3D11ShaderResourceView* g_shadowSrv = nullptr;
		// Skyrim's sun cascades, copied the moment Skyrim finishes rendering them, with each
		// cascade's transform and split as they were then (SkyCraft's CascadeSnapshot).
		ID3D11Texture2D* g_sunShadowCopy = nullptr;
		ID3D11ShaderResourceView* g_sunShadowCopySrv = nullptr;
		struct CascadeSnapshot
		{
			float m[4][4];
			float split;
			uint32_t slice;
		};
		CascadeSnapshot g_cascades[2]{};
		uint32_t g_cascadeCount = 0;
		int64_t g_cascadeQpc = 0;
		std::atomic<uint32_t> g_cascadeCaptures{ 0 };
		// The shadow mask probe (16 samples, read back a few frames later without waiting).
		ID3D11Texture2D* g_maskProbeTex = nullptr;
		ID3D11RenderTargetView* g_maskProbeRtv = nullptr;
		ID3D11Texture2D* g_maskProbeStaging = nullptr;
		bool g_maskProbePending = false;
		int64_t g_maskProbeQpc = 0;
		bool g_maskUsable = false;
		// Step N1: the NPC shadow probe (16 points, 2 pixels each), read back without waiting.
		ID3D11PixelShader* g_shadowProbePs = nullptr;
		ID3D11Texture2D* g_shadowProbeTex = nullptr;
		ID3D11RenderTargetView* g_shadowProbeRtv = nullptr;
		ID3D11Texture2D* g_shadowProbeStaging = nullptr;
		bool g_shadowProbePending = false;
		int64_t g_shadowProbeQpc = 0;
		struct ProbeTarget
		{
			uint32_t entity = 0;
			float distance = 0, height = 0;
		};
		ProbeTarget g_probeNext[kProbeCharacters]{}, g_probeSent[kProbeCharacters]{};
		uint32_t g_probeNextCount = 0, g_probeSentCount = 0;
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
		// Each entity's matrices as drawn on the last Skyrim frame (motion vectors).
		std::unordered_map<uint32_t, std::vector<GmskyRndMatrix>> g_lastMatrices;

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
			uint64_t triangles = 0, drawCalls = 0, blendedCalls = 0, framesDrawn = 0;
			uint64_t modelUploads = 0, textureUploads = 0, uploadFailures = 0;
			double uploadMsMax = 0;
			size_t gpuModels = 0, gpuTextures = 0;
			uint64_t gmodFrame = 0;
			uint32_t frameEpoch = 0, originEpoch = 0;
			bool firstValid = false;
			double first[3]{};  // the first object's first matrix's translation, in Skyrim's world
			bool firstOnScreen = false;
			float firstX = 0, firstY = 0, firstDistance = 0;
			// G6: held objects
			uint64_t heldDrawn = 0, heldNoView = 0, heldFrames = 0;
			double heldDegSum = 0, heldDegMax = 0, heldUnitsSum = 0, heldUnitsMax = 0, heldAgeSum = 0;
			// G5a
			uint32_t pointLights = 0;
			// G5b bundle
			uint64_t shadowFrames = 0, shadowCasters = 0, overlayFrames = 0, skyrimCascadeFrames = 0, normalMapCalls = 0, phongCalls = 0;
			uint32_t switches = 0;
			bool maskUsable = false;
			const char* shadowWhy = "";
			// Step N1
			uint64_t contactBlobs = 0, contactFrames = 0;
			uint64_t selfIllumCalls = 0;  // test 88
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
			ID3DBlob *shadowVsBlob = nullptr, *shadowPsBlob = nullptr, *fullVsBlob = nullptr, *overlayPsBlob = nullptr, *maskPsBlob = nullptr, *probePsBlob = nullptr;
			if (!Compile("CubeVS", "vs_5_0", &cubeVsBlob) || !Compile("CubePS", "ps_5_0", &cubePsBlob) || !Compile("ModelVS", "vs_5_0", &modelVsBlob) ||
				!Compile("ModelPS", "ps_5_0", &modelPsBlob) || !Compile("ShadowVS", "vs_5_0", &shadowVsBlob) || !Compile("ShadowPS", "ps_5_0", &shadowPsBlob) ||
				!Compile("FullVS", "vs_5_0", &fullVsBlob) || !Compile("OverlayPS", "ps_5_0", &overlayPsBlob) || !Compile("MaskProbePS", "ps_5_0", &maskPsBlob) ||
				!Compile("ShadowProbePS", "ps_5_0", &probePsBlob)) {
				for (auto* b : { cubeVsBlob, cubePsBlob, modelVsBlob, modelPsBlob, shadowVsBlob, shadowPsBlob, fullVsBlob, overlayPsBlob, maskPsBlob, probePsBlob }) {
					if (b) {
						b->Release();
					}
				}
				g_initFailed = true;
				return false;
			}
			a_device->CreateVertexShader(shadowVsBlob->GetBufferPointer(), shadowVsBlob->GetBufferSize(), nullptr, &g_shadowVs);
			a_device->CreatePixelShader(shadowPsBlob->GetBufferPointer(), shadowPsBlob->GetBufferSize(), nullptr, &g_shadowPs);
			a_device->CreateVertexShader(fullVsBlob->GetBufferPointer(), fullVsBlob->GetBufferSize(), nullptr, &g_fullVs);
			a_device->CreatePixelShader(overlayPsBlob->GetBufferPointer(), overlayPsBlob->GetBufferSize(), nullptr, &g_overlayPs);
			a_device->CreatePixelShader(maskPsBlob->GetBufferPointer(), maskPsBlob->GetBufferSize(), nullptr, &g_maskProbePs);
			Release(shadowVsBlob);
			Release(shadowPsBlob);
			Release(fullVsBlob);
			Release(overlayPsBlob);
			Release(maskPsBlob);
			a_device->CreatePixelShader(probePsBlob->GetBufferPointer(), probePsBlob->GetBufferSize(), nullptr, &g_shadowProbePs);
			Release(probePsBlob);
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
				{ "TANGENT", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, offsetof(GmskyRndVertex, tangent), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0, offsetof(GmskyRndVertex, u), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "BLENDINDICES", 0, DXGI_FORMAT_R8G8B8A8_UINT, 0, offsetof(GmskyRndVertex, bones), D3D11_INPUT_PER_VERTEX_DATA, 0 },
				{ "BLENDWEIGHT", 0, DXGI_FORMAT_R8G8B8A8_UNORM, 0, offsetof(GmskyRndVertex, weights), D3D11_INPUT_PER_VERTEX_DATA, 0 },
			};
			a_device->CreateInputLayout(modelElements, 6, modelVsBlob->GetBufferPointer(), modelVsBlob->GetBufferSize(), &g_modelLayout);
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
			rd.CullMode = D3D11_CULL_NONE;  // Source's face winding not checked yet; back faces are lit from our side
			rd.DepthClipEnable = TRUE;
			a_device->CreateRasterizerState(&rd, &g_raster);
			rd.DepthClipEnable = FALSE;  // shadow casters beyond the map's depth range still cast (SkyCraft)
			a_device->CreateRasterizerState(&rd, &g_shadowRaster);

			D3D11_DEPTH_STENCIL_DESC dd{};
			dd.DepthEnable = TRUE;
			dd.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
			dd.DepthFunc = D3D11_COMPARISON_LESS_EQUAL;
			a_device->CreateDepthStencilState(&dd, &g_depthWrite);
			dd.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ZERO;
			a_device->CreateDepthStencilState(&dd, &g_depthRead);
			dd.DepthFunc = D3D11_COMPARISON_GREATER_EQUAL;
			a_device->CreateDepthStencilState(&dd, &g_depthReadRev);
			dd.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
			a_device->CreateDepthStencilState(&dd, &g_depthWriteRev);

			// Solid: colour RGB (Skyrim's alpha left alone), every channel of the motion vectors / normals.
			D3D11_BLEND_DESC sb{};
			sb.IndependentBlendEnable = TRUE;
			sb.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_RED | D3D11_COLOR_WRITE_ENABLE_GREEN | D3D11_COLOR_WRITE_ENABLE_BLUE;
			for (int k = 1; k < 8; ++k) {
				sb.RenderTarget[k].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
			}
			a_device->CreateBlendState(&sb, &g_solidBlend);
			// See-through: over what's there by alpha; additive: added (both on the colour only).
			D3D11_BLEND_DESC ab{};
			ab.RenderTarget[0].BlendEnable = TRUE;
			ab.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
			ab.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
			ab.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
			ab.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ZERO;
			ab.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
			ab.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
			ab.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_RED | D3D11_COLOR_WRITE_ENABLE_GREEN | D3D11_COLOR_WRITE_ENABLE_BLUE;
			a_device->CreateBlendState(&ab, &g_alphaBlend);
			ab.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
			ab.RenderTarget[0].DestBlend = D3D11_BLEND_ONE;
			a_device->CreateBlendState(&ab, &g_addBlend);
			ab.RenderTarget[0].SrcBlend = D3D11_BLEND_ZERO;  // result = what's there x our colour (shadows on Skyrim)
			ab.RenderTarget[0].DestBlend = D3D11_BLEND_SRC_COLOR;
			a_device->CreateBlendState(&ab, &g_multiplyBlend);
			D3D11_BLEND_DESC nb{};
			nb.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
			a_device->CreateBlendState(&nb, &g_noBlend);

			D3D11_SAMPLER_DESC smp{};
			smp.Filter = D3D11_FILTER_ANISOTROPIC;
			smp.MaxAnisotropy = 8;
			smp.AddressU = smp.AddressV = smp.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
			smp.MaxLOD = D3D11_FLOAT32_MAX;
			a_device->CreateSamplerState(&smp, &g_sampler);
			smp.Filter = D3D11_FILTER_MIN_MAG_MIP_POINT;
			smp.AddressU = smp.AddressV = smp.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
			a_device->CreateSamplerState(&smp, &g_pointSampler);
			smp.Filter = D3D11_FILTER_COMPARISON_MIN_MAG_LINEAR_MIP_POINT;
			smp.ComparisonFunc = D3D11_COMPARISON_LESS_EQUAL;
			a_device->CreateSamplerState(&smp, &g_shadowSampler);

			// The shadow mask probe: 16 samples into a 16x1 float target, read back later.
			D3D11_TEXTURE2D_DESC pd{};
			pd.Width = 16;
			pd.Height = 1;
			pd.MipLevels = pd.ArraySize = 1;
			pd.Format = DXGI_FORMAT_R32_FLOAT;
			pd.SampleDesc.Count = 1;
			pd.Usage = D3D11_USAGE_DEFAULT;
			pd.BindFlags = D3D11_BIND_RENDER_TARGET;
			a_device->CreateTexture2D(&pd, nullptr, &g_maskProbeTex);
			if (g_maskProbeTex) {
				a_device->CreateRenderTargetView(g_maskProbeTex, nullptr, &g_maskProbeRtv);
			}
			pd.Usage = D3D11_USAGE_STAGING;
			pd.BindFlags = 0;
			pd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
			a_device->CreateTexture2D(&pd, nullptr, &g_maskProbeStaging);
			// Step N1: the NPC shadow probe, 32 x 1 float4.
			pd.Width = 2 * kProbePoints;
			pd.Format = DXGI_FORMAT_R32G32B32A32_FLOAT;
			pd.Usage = D3D11_USAGE_DEFAULT;
			pd.BindFlags = D3D11_BIND_RENDER_TARGET;
			pd.CPUAccessFlags = 0;
			a_device->CreateTexture2D(&pd, nullptr, &g_shadowProbeTex);
			if (g_shadowProbeTex) {
				a_device->CreateRenderTargetView(g_shadowProbeTex, nullptr, &g_shadowProbeRtv);
			}
			pd.Usage = D3D11_USAGE_STAGING;
			pd.BindFlags = 0;
			pd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
			a_device->CreateTexture2D(&pd, nullptr, &g_shadowProbeStaging);

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
			                g_depthWrite && g_depthWriteRev && g_depthRead && g_depthReadRev && g_solidBlend && g_alphaBlend && g_addBlend && g_sampler && g_whiteSrv &&
			                g_shadowVs && g_shadowPs && g_fullVs && g_overlayPs && g_maskProbePs && g_pointSampler && g_shadowSampler && g_shadowRaster && g_multiplyBlend &&
			                g_noBlend && g_maskProbeRtv && g_maskProbeStaging && g_shadowProbePs && g_shadowProbeRtv && g_shadowProbeStaging;
			if (!ok) {
				logger::error("in-frame drawing: failed to create its graphics objects");
				g_initFailed = true;
				return false;
			}
			g_device = a_device;
			g_device->AddRef();
			logger::info("in-frame drawing: ready (test cube {}, model drawing on, Skyrim's light, held objects follow the camera, shadows, normal maps, phong, contact shadows)", kTestCube ? "on" : "off");
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
			D3D11_TEXTURE2D_DESC sd{}, dd{}, md{}, nd{};
			a_sceneTex->GetDesc(&sd);
			a_depthTex->GetDesc(&dd);
			const auto& motion = a_rd.renderTargets[RE::RENDER_TARGETS::kMOTION_VECTOR];
			if (motion.texture) {
				reinterpret_cast<ID3D11Texture2D*>(motion.texture)->GetDesc(&md);
			}
			const auto& normals = a_rd.renderTargets[RE::RENDER_TARGETS::kNORMAL_TAAMASK_SSRMASK];
			if (normals.texture) {
				reinterpret_cast<ID3D11Texture2D*>(normals.texture)->GetDesc(&nd);
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
			logger::info("in-frame drawing: scene {}x{} format {} ({}), depth {}x{} format {}, motion vectors {}x{} format {}, normals {}x{} format {}; bound here: targets [ {}] depth {}; {}",
				sd.Width, sd.Height, int(sd.Format), hdr ? "HDR" : "not HDR", dd.Width, dd.Height, int(dd.Format), md.Width, md.Height, int(md.Format), nd.Width, nd.Height,
				int(nd.Format), slots, boundDsv ? "yes" : "none", g_targetsOk ? "drawing here" : "UNEXPECTED, nothing drawn");
			return g_targetsOk;
		}

		// ---- G5a: Skyrim's light (SkyCraft's GatherLighting, MIT) ----

		void Set4(float a_out[4], float a_x, float a_y, float a_z, float a_w)
		{
			a_out[0] = a_x, a_out[1] = a_y, a_out[2] = a_z, a_out[3] = a_w;
		}

		void SetColor(float a_out[4], const RE::Color& a_c, float a_w = 0.0f)
		{
			Set4(a_out, a_c.red / 255.0f, a_c.green / 255.0f, a_c.blue / 255.0f, a_w);
		}

		// An interior cell's lighting values, from its lighting template where the cell inherits them.
		template <class F>
		void InteriorValue(RE::TESObjectCELL* a_cell, RE::INTERIOR_DATA::Inherit a_flag, F a_get)
		{
			auto* own = a_cell->GetLighting();
			auto* tmpl = a_cell->GetRuntimeData().lightingTemplate;
			const bool inherit = tmpl && own->lightingTemplateInheritanceFlags.any(a_flag);
			a_get(inherit ? tmpl->data : *own, inherit ? tmpl : nullptr);
		}

		// The sun, moon or interior directional light, the directional ambient, the nearest point
		// lights (torches, fires, spells) and fog, as Skyrim lights its own geometry.
		void GatherLighting(FrameConstants& a_fc, const RE::NiPoint3& a_cam, Seen& a_seen)
		{
			// Fallbacks, should any of Skyrim's lighting be missing: plain daylight.
			Set4(a_fc.sunDir, 0.3f, -0.4f, 0.87f, 0.0f);
			Set4(a_fc.sunColor, 0.9f, 0.85f, 0.75f, 0.0f);
			for (auto& a : a_fc.ambient) {
				Set4(a, 0.45f, 0.47f, 0.5f, 0.0f);
			}
			Set4(a_fc.fogNear, 0, 0, 0, 0);

			auto* player = RE::PlayerCharacter::GetSingleton();
			auto* cell = player ? player->GetParentCell() : nullptr;
			const bool interior = cell && cell->IsInteriorCell() && cell->GetLighting();
			auto* sky = RE::Sky::GetSingleton();
			auto* ssn = RE::BSShaderManager::State::GetSingleton().shadowSceneNode[0];

			RE::NiDirectionalLight* dirLight = nullptr;
			if (ssn) {
				auto* bsSun = ssn->GetRuntimeData().sunLight;
				if (bsSun && bsSun->light) {
					dirLight = netimmerse_cast<RE::NiDirectionalLight*>(bsSun->light.get());
				}
			}
			float sunFade = 1.0f;
			if (dirLight) {
				const auto dir = dirLight->GetWorldDirection();
				const float len = dir.Length();
				if (len > 1e-4f) {
					Set4(a_fc.sunDir, -dir.x / len, -dir.y / len, -dir.z / len, 0.0f);
				}
				const auto& ld = dirLight->GetLightRuntimeData();
				sunFade = ld.fade > 0.0f && ld.fade < 16.0f ? ld.fade : 1.0f;
				Set4(a_fc.sunColor, ld.diffuse.red * sunFade, ld.diffuse.green * sunFade, ld.diffuse.blue * sunFade, 0.0f);
			}

			// Directional ambient ("DALC"): six colours named by the way the light travels, so a
			// face looking along +Z (up) takes the Z- colour, the sky's light coming down (SkyCraft,
			// checked in its game).
			using Inherit = RE::INTERIOR_DATA::Inherit;
			if (interior) {
				InteriorValue(cell, Inherit::kAmbientColor, [&](const RE::INTERIOR_DATA& a_d, RE::BGSLightingTemplate* a_t) {
					const auto& dal = a_t ? a_t->directionalAmbientLightingColors.directional : a_d.directionalAmbientLightingColors.directional;
					SetColor(a_fc.ambient[0], dal.x.min);
					SetColor(a_fc.ambient[1], dal.x.max);
					SetColor(a_fc.ambient[2], dal.y.min);
					SetColor(a_fc.ambient[3], dal.y.max);
					SetColor(a_fc.ambient[4], dal.z.min);
					SetColor(a_fc.ambient[5], dal.z.max);
				});
			} else if (sky) {
				for (int axis = 0; axis < 3; ++axis) {
					const auto& plus = sky->directionalAmbientColors[axis][0];
					const auto& minus = sky->directionalAmbientColors[axis][1];
					Set4(a_fc.ambient[axis * 2], minus.red, minus.green, minus.blue, 0.0f);  // faces looking +axis
					Set4(a_fc.ambient[axis * 2 + 1], plus.red, plus.green, plus.blue, 0.0f);
				}
			}

			// Fog, as Skyrim fades its own geometry with distance.
			float fogNearD = 0, fogFarD = 0, fogPower = 1, fogMax = 0;
			if (interior) {
				InteriorValue(cell, Inherit::kFogNear, [&](const RE::INTERIOR_DATA& a_d, auto*) { fogNearD = a_d.fogNear; });
				InteriorValue(cell, Inherit::kFogFar, [&](const RE::INTERIOR_DATA& a_d, auto*) { fogFarD = a_d.fogFar; });
				InteriorValue(cell, Inherit::kFogPower, [&](const RE::INTERIOR_DATA& a_d, auto*) { fogPower = a_d.fogPower; });
				InteriorValue(cell, Inherit::kFogMax, [&](const RE::INTERIOR_DATA& a_d, auto*) { fogMax = a_d.fogClamp; });
				InteriorValue(cell, Inherit::kFogColor, [&](const RE::INTERIOR_DATA& a_d, auto*) {
					SetColor(a_fc.fogNear, a_d.fogColorNear);
					SetColor(a_fc.fogFar, a_d.fogColorFar);
				});
			} else if (sky) {
				fogNearD = sky->fogNear, fogFarD = sky->fogFar, fogPower = sky->fogPower, fogMax = sky->fogClamp;
				const auto& n = sky->skyColor[RE::TESWeather::ColorTypes::kFogNear];
				const auto& f = sky->skyColor[RE::TESWeather::ColorTypes::kFogFar];
				Set4(a_fc.fogNear, n.red, n.green, n.blue, 0.0f);
				Set4(a_fc.fogFar, f.red, f.green, f.blue, 0.0f);
			}
			if (fogFarD > fogNearD + 1.0f && fogMax > 0.0f) {
				Set4(a_fc.fogRange, 1.0f / (fogFarD - fogNearD), fogNearD / (fogFarD - fogNearD), fogPower > 0.0f ? fogPower : 1.0f, std::min(fogMax, 1.0f));
				a_fc.fogNear[3] = 1.0f;
			}

			// The point lights nearest to reaching the camera.
			struct Candidate
			{
				float reach;
				RE::NiPoint3 pos;
				float radius;
				RE::NiColor color;
			};
			static std::vector<Candidate> candidates;
			candidates.clear();
			auto consider = [&](RE::BSLight* a_light) {
				if (!a_light || !a_light->pointLight || !a_light->light) {
					return;
				}
				auto* light = a_light->light.get();
				const auto& ld = light->GetLightRuntimeData();
				const float radius = ld.radius.x;
				if (!(radius > 1.0f) || light->GetFlags().any(RE::NiAVObject::Flag::kHidden)) {
					return;
				}
				const float dimmer = std::clamp(a_light->lodDimmer, 0.0f, 1.0f) * ld.fade;
				const auto pos = light->world.translate;
				const float reach = pos.GetDistance(a_cam) - radius;
				if (reach > 6000.0f || std::fabs(dimmer) < 1e-3f) {
					return;
				}
				candidates.push_back({ reach, pos, radius, { ld.diffuse.red * dimmer, ld.diffuse.green * dimmer, ld.diffuse.blue * dimmer } });
			};
			if (ssn) {
				auto& rd = ssn->GetRuntimeData();
				for (auto& light : rd.activeLights) {
					consider(light.get());
				}
				for (auto& light : rd.activeShadowLights) {
					consider(light.get());
				}
			}
			std::ranges::sort(candidates, {}, &Candidate::reach);
			const auto count = std::min<std::size_t>(candidates.size(), kMaxLights);
			for (std::size_t k = 0; k < count; ++k) {
				const auto& c = candidates[k];
				Set4(a_fc.lightPos[k], c.pos.x - a_cam.x, c.pos.y - a_cam.y, c.pos.z - a_cam.z, 1.0f / c.radius);
				Set4(a_fc.lightColor[k], c.color.red, c.color.green, c.color.blue, 0.0f);
			}
			a_fc.sunColor[3] = float(count);
			a_seen.pointLights = uint32_t(count);
			a_fc.shadowParams[0] = (!interior && dirLight && a_fc.sunDir[2] > 0.03f) ? 1.0f : 0.0f;
			a_seen.shadowWhy = interior ? "interior" : !dirLight ? "no directional light" : a_fc.sunDir[2] <= 0.03f ? "sun below the horizon" : "";

			// What Skyrim gave us, now and then, for tuning.
			static RE::TESObjectCELL* loggedCell = nullptr;
			static int64_t lastLog = 0;
			const auto now = Qpc();
			if (cell != loggedCell || QpcMs(now - lastLog) > 30000.0) {
				loggedCell = cell;
				lastLog = now;
				const float* up = a_fc.ambient[4];
				const float* down = a_fc.ambient[5];
				const float* east = a_fc.ambient[0];
				logger::info("lighting ({}{}): sun dir ({:.2f} {:.2f} {:.2f}) colour ({:.2f} {:.2f} {:.2f}) fade {:.2f}; ambient up ({:.2f} {:.2f} {:.2f}) down ({:.2f} {:.2f} {:.2f}) east ({:.2f} {:.2f} {:.2f}); {} point lights; fog {} ({:.0f}-{:.0f}, power {:.2f}, max {:.2f})",
					interior ? "interior" : "exterior", dirLight ? "" : ", no directional light", a_fc.sunDir[0], a_fc.sunDir[1], a_fc.sunDir[2], a_fc.sunColor[0],
					a_fc.sunColor[1], a_fc.sunColor[2], sunFade, up[0], up[1], up[2], down[0], down[1], down[2], east[0], east[1], east[2], count,
					a_fc.fogNear[3] > 0.5f ? "on" : "off", fogNearD, fogFarD, fogPower, fogMax);
			}
		}

		// ---- G5b: shadows (SkyCraft's, MIT) ----

		bool EnsureShadowMap()
		{
			if (g_shadowDsv) {
				return true;
			}
			D3D11_TEXTURE2D_DESC td{};
			td.Width = td.Height = kShadowSize;
			td.MipLevels = 1;
			td.ArraySize = 1;
			td.Format = DXGI_FORMAT_R32_TYPELESS;
			td.SampleDesc.Count = 1;
			td.Usage = D3D11_USAGE_DEFAULT;
			td.BindFlags = D3D11_BIND_DEPTH_STENCIL | D3D11_BIND_SHADER_RESOURCE;
			D3D11_DEPTH_STENCIL_VIEW_DESC dv{};
			dv.Format = DXGI_FORMAT_D32_FLOAT;
			dv.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2D;
			D3D11_SHADER_RESOURCE_VIEW_DESC sv{};
			sv.Format = DXGI_FORMAT_R32_FLOAT;
			sv.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
			sv.Texture2D.MipLevels = 1;
			if (FAILED(g_device->CreateTexture2D(&td, nullptr, &g_shadowTex)) || FAILED(g_device->CreateDepthStencilView(g_shadowTex, &dv, &g_shadowDsv)) ||
				FAILED(g_device->CreateShaderResourceView(g_shadowTex, &sv, &g_shadowSrv))) {
				logger::error("shadows: our sun shadow map {}x{} failed", kShadowSize, kShadowSize);
				Release(g_shadowSrv);
				Release(g_shadowDsv);
				Release(g_shadowTex);
				return false;
			}
			logger::info("shadows: our sun shadow map ready ({}x{}, {:.0f} Skyrim units across)", kShadowSize, kShadowSize, 2.0f * kShadowRadius);
			return true;
		}

		// An orthographic view down the sun's rays, centred on the camera and snapped to whole
		// shadow texels so shadow edges hold still as the camera moves; it follows the sun in
		// quarter-degree steps (SkyCraft's SetSunShadowMatrix).
		void SetSunShadowMatrix(FrameConstants& a_fc, const RE::NiPoint3& a_cam)
		{
			static double stable[3] = { 0.0, 0.0, 0.0 };
			const double now[3] = { -a_fc.sunDir[0], -a_fc.sunDir[1], -a_fc.sunDir[2] };  // the way sunlight travels
			const double cosStep = stable[0] * now[0] + stable[1] * now[1] + stable[2] * now[2];
			if (cosStep < 0.99999) {
				std::memcpy(stable, now, sizeof(stable));
			}
			const double f[3] = { stable[0], stable[1], stable[2] };
			const double up[3] = { 0.0, std::fabs(f[2]) > 0.99 ? 1.0 : 0.0, std::fabs(f[2]) > 0.99 ? 0.0 : 1.0 };
			double r[3] = { f[1] * up[2] - f[2] * up[1], f[2] * up[0] - f[0] * up[2], f[0] * up[1] - f[1] * up[0] };
			const double rl = std::sqrt(r[0] * r[0] + r[1] * r[1] + r[2] * r[2]);
			for (auto& c : r) {
				c /= rl;
			}
			const double u[3] = { r[1] * f[2] - r[2] * f[1], r[2] * f[0] - r[0] * f[2], r[0] * f[1] - r[1] * f[0] };
			const double texel = 2.0 * kShadowRadius / kShadowSize;
			const double a = r[0] * a_cam.x + r[1] * a_cam.y + r[2] * a_cam.z;
			const double b = u[0] * a_cam.x + u[1] * a_cam.y + u[2] * a_cam.z;
			const double offX = a - std::floor(a / texel) * texel, offY = b - std::floor(b / texel) * texel;
			auto& m = a_fc.lightViewProj;
			for (int k = 0; k < 3; ++k) {
				m[0][k] = float(r[k] / kShadowRadius);
				m[1][k] = float(u[k] / kShadowRadius);
				m[2][k] = float(f[k] / (2.0 * kShadowDepth));
				m[3][k] = 0.0f;
			}
			m[0][3] = float(offX / kShadowRadius);
			m[1][3] = float(offY / kShadowRadius);
			m[2][3] = 0.5f;
			m[3][3] = 1.0f;
			Set4(a_fc.shadowParams, 1.0f, float(texel * 1.5), 1.0f / kShadowSize, float(3.0 / (2.0 * kShadowDepth)));
			a_fc.shadowExtra[1] = (kShadowRadius * 1.5f) * (kShadowRadius * 1.5f);
		}

		DXGI_FORMAT DepthReadFormat(DXGI_FORMAT a_typeless)
		{
			switch (a_typeless) {
			case DXGI_FORMAT_R16_TYPELESS:
			case DXGI_FORMAT_D16_UNORM:
				return DXGI_FORMAT_R16_UNORM;
			case DXGI_FORMAT_R24G8_TYPELESS:
			case DXGI_FORMAT_D24_UNORM_S8_UINT:
				return DXGI_FORMAT_R24_UNORM_X8_TYPELESS;
			case DXGI_FORMAT_R32_TYPELESS:
			case DXGI_FORMAT_D32_FLOAT:
				return DXGI_FORMAT_R32_FLOAT;
			case DXGI_FORMAT_R32G8X24_TYPELESS:
				return DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS;
			default:
				return DXGI_FORMAT_UNKNOWN;
			}
		}

		// Right after Skyrim renders the sun's shadow maps: copy its first two cascades and their
		// transforms (SkyCraft's CaptureSunShadows).
		void CaptureSunShadows(RE::BSShadowLight* a_sun)
		{
			auto* renderer = RE::BSGraphics::Renderer::GetSingleton();
			if (!g_device || !renderer || !a_sun) {
				return;
			}
			auto& descs = a_sun->GetRuntimeData().shadowmapDescriptors;
			const auto count = std::min<std::uint32_t>(descs.size(), 2);
			if (count == 0) {
				return;
			}
			auto* context = reinterpret_cast<ID3D11DeviceContext*>(renderer->GetRuntimeData().context);
			// Where they went: the depth buffer still bound, else the shared shadow map array.
			ID3D11Resource* src = nullptr;
			ID3D11DepthStencilView* bound = nullptr;
			context->OMGetRenderTargets(0, nullptr, &bound);
			if (bound) {
				bound->GetResource(&src);
				Release(bound);
			}
			if (!src) {
				auto* tex = reinterpret_cast<ID3D11Texture2D*>(renderer->GetDepthStencilData().depthStencils[RE::RENDER_TARGETS_DEPTHSTENCIL::kSHADOWMAPS].texture);
				if (tex) {
					tex->AddRef();
					src = tex;
				}
			}
			if (!src) {
				return;
			}
			D3D11_TEXTURE2D_DESC sd{};
			static_cast<ID3D11Texture2D*>(src)->GetDesc(&sd);
			if (!g_sunShadowCopy) {
				D3D11_TEXTURE2D_DESC cd = sd;
				cd.ArraySize = 2;
				cd.MipLevels = 1;
				cd.BindFlags = D3D11_BIND_SHADER_RESOURCE;
				cd.Usage = D3D11_USAGE_DEFAULT;
				cd.CPUAccessFlags = 0;
				cd.MiscFlags = 0;
				D3D11_SHADER_RESOURCE_VIEW_DESC vd{};
				vd.Format = DepthReadFormat(sd.Format);
				vd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DARRAY;
				vd.Texture2DArray.MipLevels = 1;
				vd.Texture2DArray.ArraySize = 2;
				const bool ok = vd.Format != DXGI_FORMAT_UNKNOWN && sd.SampleDesc.Count == 1 && SUCCEEDED(g_device->CreateTexture2D(&cd, nullptr, &g_sunShadowCopy)) &&
				                SUCCEEDED(g_device->CreateShaderResourceView(g_sunShadowCopy, &vd, &g_sunShadowCopySrv));
				logger::info("shadows: capturing Skyrim's sun cascades from a {}x{} x{} format {} depth array ({})", sd.Width, sd.Height, sd.ArraySize,
					static_cast<int>(sd.Format), ok ? "ok" : "can't read that format");
				if (!ok) {
					Release(g_sunShadowCopySrv);
					Release(g_sunShadowCopy);
					Release(src);
					return;
				}
			}
			D3D11_TEXTURE2D_DESC cd{};
			g_sunShadowCopy->GetDesc(&cd);
			if (cd.Width != sd.Width || cd.Height != sd.Height || cd.Format != sd.Format) {
				Release(g_sunShadowCopySrv);
				Release(g_sunShadowCopy);
				Release(src);
				return;  // made again next time
			}
			const auto& dsl = static_cast<RE::BSShadowDirectionalLight*>(a_sun)->GetShadowDirectionalLightRuntimeData();
			for (std::uint32_t k = 0; k < count; ++k) {
				const UINT slice = descs[k].shadowmapIndex;
				if (slice < sd.ArraySize) {
					context->CopySubresourceRegion(g_sunShadowCopy, D3D11CalcSubresource(0, k, 1), 0, 0, 0, src, D3D11CalcSubresource(0, slice, sd.MipLevels), nullptr);
				}
				std::memcpy(g_cascades[k].m, &descs[k].lightTransform.m, sizeof(g_cascades[k].m));
				g_cascades[k].split = k < 3 ? dsl.endSplitDistances[k] : 0.0f;
				g_cascades[k].slice = slice;
			}
			g_cascadeCount = count;
			g_cascadeQpc = Qpc();
			g_cascadeCaptures.fetch_add(1, std::memory_order_relaxed);
			Release(src);
		}

		// Skyrim's sun cascades for this frame, re-based on the camera (SkyCraft's SetSkyrimShadows).
		// Returns the copied shadow maps to read, or null.
		ID3D11ShaderResourceView* SetSkyrimShadows(FrameConstants& a_fc, const RE::NiPoint3& a_cam)
		{
			a_fc.skyShadowSplits[3] = 0.0f;
			if (!g_sunShadowCopySrv || g_cascadeCount == 0 || QpcMs(Qpc() - g_cascadeQpc) > 1000.0) {
				return nullptr;  // not captured lately (Skyrim doesn't redraw them every frame, but within a second)
			}
			D3D11_TEXTURE2D_DESC td{};
			g_sunShadowCopy->GetDesc(&td);
			bool standard = true;
			for (std::uint32_t k = 0; k < g_cascadeCount; ++k) {
				const auto& m = g_cascades[k].m;
				// Row-vector convention (v * M, translation in the last row) unless the matrix says otherwise.
				const bool rowVector = std::fabs(m[0][3]) + std::fabs(m[1][3]) + std::fabs(m[2][3]) < 1e-6f;
				auto at = [&](int a_r, int a_c) { return double(rowVector ? m[a_r][a_c] : m[a_c][a_r]); };  // as v * M
				for (int out = 0; out < 4; ++out) {
					for (int in = 0; in < 3; ++in) {
						a_fc.skyShadowProj[k][out][in] = float(at(in, out));
					}
					a_fc.skyShadowProj[k][out][3] = float(at(3, out) + at(0, out) * a_cam.x + at(1, out) * a_cam.y + at(2, out) * a_cam.z);
				}
				if (k == 0) {
					// Depth grows along the sunlight (standard) or against it (reversed).
					const double g = at(0, 2) * -a_fc.sunDir[0] + at(1, 2) * -a_fc.sunDir[1] + at(2, 2) * -a_fc.sunDir[2];
					standard = g > 0.0;
					static bool logged = false;
					if (!logged) {
						logged = true;
						const float u = a_fc.skyShadowProj[0][0][3], v = a_fc.skyShadowProj[0][1][3], z = a_fc.skyShadowProj[0][2][3], w = a_fc.skyShadowProj[0][3][3];
						logger::info("shadows: Skyrim's sun: {} cascades, slices {} {}, maps {}x{} format {}, splits {:.0f} {:.0f}; matrix {}, depth {}; camera at uv ({:.3f}, {:.3f}) depth {:.4f} w {:.3f}",
							g_cascadeCount, g_cascades[0].slice, g_cascadeCount > 1 ? g_cascades[1].slice : 0, td.Width, td.Height, static_cast<int>(td.Format),
							g_cascades[0].split, g_cascades[1].split, rowVector ? "row-vector" : "column-vector", standard ? "standard" : "reversed", u / w, v / w, z / w, w);
					}
				}
			}
			a_fc.skyShadowSplits[0] = g_cascades[0].split;
			a_fc.skyShadowSplits[1] = g_cascadeCount > 1 ? g_cascades[1].split : g_cascades[0].split;
			a_fc.skyShadowSplits[3] = float(g_cascadeCount);
			Set4(a_fc.skyShadowParams, 0.0f, g_cascadeCount > 1 ? 1.0f : 0.0f, 1.0f / float(std::max<UINT>(td.Width, 1)), standard ? 1.0f : -1.0f);
			return g_sunShadowCopySrv;
		}

		// 4x4 inverse (row-major), for turning Skyrim's depth back into positions (SkyCraft's Invert).
		bool Invert(const float a_m[4][4], float a_out[4][4])
		{
			double m[16], inv[16];
			for (int k = 0; k < 16; ++k) {
				m[k] = a_m[k / 4][k % 4];
			}
			inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
			inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
			inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
			inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
			inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
			inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
			inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
			inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
			inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
			inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
			inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
			inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
			inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
			inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
			inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
			inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
			const double det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
			if (std::fabs(det) < 1e-30) {
				return false;
			}
			for (int k = 0; k < 16; ++k) {
				a_out[k / 4][k % 4] = float(inv[k] / det);
			}
			return true;
		}

		// Skyrim's shadow mask: probed every 5 s (16 samples, read back without waiting); used only
		// if it holds 0-1 values with some lit (SkyCraft's ReadProbe).
		void MaskProbe(ID3D11DeviceContext* a_context, ID3D11ShaderResourceView* a_mask)
		{
			if (g_maskProbePending) {
				D3D11_MAPPED_SUBRESOURCE mapped{};
				if (a_context->Map(g_maskProbeStaging, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, &mapped) == S_OK) {
					std::array<float, 16> v{};
					std::memcpy(v.data(), mapped.pData, sizeof(v));
					a_context->Unmap(g_maskProbeStaging, 0);
					g_maskProbePending = false;
					std::ranges::sort(v);
					const bool usable = v[0] >= -0.001f && v[15] <= 1.001f && v[15] > 0.5f;
					if (usable != g_maskUsable || g_maskProbeQpc == 0) {
						logger::info("shadows: Skyrim's shadow mask probe: min {:.3f} median {:.3f} max {:.3f} -> {}", v[0], v[8], v[15],
							usable ? "used (no double shadows)" : "not used");
					}
					g_maskUsable = usable;
				}
				return;
			}
			if (!a_mask || (g_maskProbeQpc && QpcMs(Qpc() - g_maskProbeQpc) < 5000.0)) {
				return;
			}
			g_maskProbeQpc = Qpc();
			D3D11_VIEWPORT vp{ 0, 0, 16, 1, 0, 1 };
			a_context->OMSetRenderTargets(1, &g_maskProbeRtv, nullptr);
			a_context->RSSetViewports(1, &vp);
			a_context->RSSetState(g_raster);
			a_context->OMSetBlendState(g_noBlend, nullptr, 0xFFFFFFFF);
			a_context->OMSetDepthStencilState(nullptr, 0);
			a_context->IASetInputLayout(nullptr);
			a_context->VSSetShader(g_fullVs, nullptr, 0);
			a_context->PSSetShader(g_maskProbePs, nullptr, 0);
			a_context->PSSetShaderResources(7, 1, &a_mask);
			a_context->PSSetSamplers(1, 1, &g_pointSampler);
			a_context->Draw(3, 0);
			a_context->CopyResource(g_maskProbeStaging, g_maskProbeTex);
			ID3D11ShaderResourceView* none = nullptr;
			a_context->PSSetShaderResources(7, 1, &none);
			g_maskProbePending = true;
		}

		// ---- Step N1: contact shadows under Garry's Mod's NPCs and players, and their sun shadow probe ----

		// The feet of Garry's Mod's NPCs, nextbots and players near the camera, nearest first, for their
		// contact shadows (SkyCraft's GatherContactShadows: radius 1.25 x max(width, half a block));
		// the nearest few also get the probe's points (feet, middle, top).
		void GatherContactShadows(FrameConstants& a_fc, const RE::NiPoint3& a_cam, Seen& a_seen)
		{
			g_probeNextCount = 0;
			const auto& frame = Render::NewestFrame();
			GmskyOrigin origin{};
			if (!frame.valid || !Link::Get().IsOpen() || !SlotRead(Link::Get().Shared().origin, origin) || origin.epoch == 0 ||
				frame.header.origin_epoch != origin.epoch) {
				return;
			}
			struct Blob
			{
				float d2;
				float v[4];
				float height;
				uint32_t entity;
			};
			static std::vector<Blob> blobs;
			blobs.clear();
			const double per = 1.0 / kGmodPerSkyrim;
			for (const auto& o : frame.objects) {
				if (!(o.flags & GMSKY_RND_OBJ_CONTACT)) {
					continue;
				}
				const float rx = float(origin.skyrim_origin[0] + o.feet[0] * per - double(a_cam.x));
				const float ry = float(origin.skyrim_origin[1] + o.feet[1] * per - double(a_cam.y));
				const float rz = float(origin.skyrim_origin[2] + o.feet[2] * per - double(a_cam.z));
				const float radius = std::max(float(o.width * per), 35.0f) * 1.25f;
				blobs.push_back({ rx * rx + ry * ry + rz * rz, { rx, ry, rz, radius }, float(o.height * per), o.entity });
			}
			std::ranges::sort(blobs, {}, &Blob::d2);
			const auto count = std::min<std::size_t>(blobs.size(), kMaxBlobs);
			for (std::size_t k = 0; k < count; ++k) {
				std::memcpy(a_fc.aoBlobs[k], blobs[k].v, sizeof(blobs[k].v));
			}
			a_fc.aoParams[0] = float(count);
			a_seen.contactBlobs += count;
			++a_seen.contactFrames;
			const auto probed = std::min<std::size_t>(count, kProbeCharacters);
			for (std::size_t k = 0; k < probed; ++k) {
				const auto& b = blobs[k];
				const float up[3] = { 10.0f, 0.5f * b.height, 0.9f * b.height };
				for (int j = 0; j < 3; ++j) {
					Set4(a_fc.probePts[k * 3 + j], b.v[0], b.v[1], b.v[2] + up[j], 1.0f);
				}
				g_probeNext[k] = { b.entity, std::sqrt(b.d2), b.height };
			}
			g_probeNextCount = uint32_t(probed);
		}

		// Every 2 s with characters near and sun shadows on: Skyrim's and our sun shadow at their
		// feet, middle and top, read back a few frames later without waiting, and logged.
		void ShadowProbe(ID3D11DeviceContext* a_context, ID3D11ShaderResourceView* a_skyrimShadows)
		{
			if (g_shadowProbePending) {
				D3D11_MAPPED_SUBRESOURCE mapped{};
				if (a_context->Map(g_shadowProbeStaging, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, &mapped) == S_OK) {
					std::array<std::array<float, 4>, 2 * kProbePoints> v{};
					std::memcpy(v.data(), mapped.pData, sizeof(v));
					a_context->Unmap(g_shadowProbeStaging, 0);
					g_shadowProbePending = false;
					for (uint32_t k = 0; k < g_probeSentCount; ++k) {
						const auto& a = v[k * 6];      // feet: (Skyrim's, ours, cascade, view depth)
						const auto& b = v[k * 6 + 2];  // middle
						const auto& c = v[k * 6 + 4];  // top
						const auto& m = v[k * 6 + 1];  // feet in Skyrim's map
						logger::info("shadows: NPC probe: entity {}, {:.0f} units away, {:.0f} tall: Skyrim's sun shadow at feet {:.2f}, middle {:.2f}, top {:.2f} "
									 "(1 = lit; cascade {:.0f}, view depth {:.0f}); ours {:.2f} {:.2f} {:.2f}; feet in Skyrim's map at uv ({:.3f}, {:.3f}) depth {:.4f}, stored {:.4f}",
							g_probeSent[k].entity, g_probeSent[k].distance, g_probeSent[k].height, a[0], b[0], c[0], a[2], a[3], a[1], b[1], c[1], m[0], m[1], m[2], m[3]);
					}
				}
				return;
			}
			if (g_probeNextCount == 0 || (g_shadowProbeQpc && QpcMs(Qpc() - g_shadowProbeQpc) < 2000.0)) {
				return;
			}
			g_shadowProbeQpc = Qpc();
			std::copy(std::begin(g_probeNext), std::end(g_probeNext), std::begin(g_probeSent));
			g_probeSentCount = g_probeNextCount;
			D3D11_VIEWPORT vp{ 0, 0, float(2 * kProbePoints), 1, 0, 1 };
			a_context->OMSetRenderTargets(1, &g_shadowProbeRtv, nullptr);
			a_context->RSSetViewports(1, &vp);
			a_context->RSSetState(g_raster);
			a_context->OMSetBlendState(g_noBlend, nullptr, 0xFFFFFFFF);
			a_context->OMSetDepthStencilState(nullptr, 0);
			a_context->IASetInputLayout(nullptr);
			a_context->VSSetShader(g_fullVs, nullptr, 0);
			a_context->PSSetShader(g_shadowProbePs, nullptr, 0);
			ID3D11ShaderResourceView* srvs[2] = { g_shadowSrv, a_skyrimShadows };
			a_context->PSSetShaderResources(4, 2, srvs);
			a_context->Draw(3, 0);
			a_context->CopyResource(g_shadowProbeStaging, g_shadowProbeTex);
			ID3D11ShaderResourceView* none[2]{};
			a_context->PSSetShaderResources(4, 2, none);
			g_shadowProbePending = true;
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
				// Read as stored, the way vanilla Skyrim reads its own colour textures (test 86; before,
				// sRGB views made them darker than Skyrim's); Skyrim makes the smaller mip levels.
				td.MipLevels = 0;
				td.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
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
			td.Format = h.format == GMSKY_RND_TEX_BC1 ? DXGI_FORMAT_BC1_UNORM : h.format == GMSKY_RND_TEX_BC2 ? DXGI_FORMAT_BC2_UNORM : DXGI_FORMAT_BC3_UNORM;
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
					logger::info("in-frame drawing: texture {} uploaded: {}x{} format {}{}", id, src.header.width, src.header.height, src.header.format,
						(src.header.flags & GMSKY_RND_TEXF_LINEAR) ? " (linear: normal map)" : "");
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

		// Matrices into a buffer the vertex shader reads (3 rows of float4 each).
		bool UploadRows(ID3D11DeviceContext* a_context, RowBuffer& a_b, const std::vector<GmskyRndMatrix>& a_m)
		{
			const UINT rows = UINT(a_m.size() * 3);
			if (rows > a_b.capacity || !a_b.buf) {
				Release(a_b.srv);
				Release(a_b.buf);
				a_b.capacity = 0;
				const UINT cap = std::max<UINT>(rows * 2, 3 * 1024);
				D3D11_BUFFER_DESC bd{};
				bd.ByteWidth = cap * 16;
				bd.Usage = D3D11_USAGE_DYNAMIC;
				bd.BindFlags = D3D11_BIND_SHADER_RESOURCE;
				bd.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
				bd.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_STRUCTURED;
				bd.StructureByteStride = 16;
				if (FAILED(g_device->CreateBuffer(&bd, nullptr, &a_b.buf)) || FAILED(g_device->CreateShaderResourceView(a_b.buf, nullptr, &a_b.srv))) {
					Release(a_b.buf);
					return false;
				}
				a_b.capacity = cap;
			}
			D3D11_MAPPED_SUBRESOURCE mapped{};
			if (FAILED(a_context->Map(a_b.buf, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
				return false;
			}
			std::memcpy(mapped.pData, a_m.data(), a_m.size() * sizeof(GmskyRndMatrix));
			a_context->Unmap(a_b.buf, 0);
			return true;
		}

		// Source's AngleVectors: forward, right, up of a view at pitch / yaw / roll (degrees).
		void SourceAxes(const float a_angles[3], double a_f[3], double a_r[3], double a_u[3])
		{
			const double p = a_angles[0] * kPi / 180.0, y = a_angles[1] * kPi / 180.0, r = a_angles[2] * kPi / 180.0;
			const double sp = std::sin(p), cp = std::cos(p), sy = std::sin(y), cy = std::cos(y), sr = std::sin(r), cr = std::cos(r);
			a_f[0] = cp * cy, a_f[1] = cp * sy, a_f[2] = -sp;
			a_r[0] = -sr * sp * cy + cr * sy, a_r[1] = -sr * sp * sy - cr * cy, a_r[2] = -sr * cp;
			a_u[0] = cr * sp * cy + sr * sy, a_u[1] = cr * sp * sy - sr * cy, a_u[2] = cr * cp;
		}

		// G6: the correction for held objects, camera-relative position then -> now: M x (p - e),
		// with M = R_skyrim_now x R_gmod_then^T and e = Garry's Mod's eye relative to Skyrim's camera.
		// Garry's Mod and Skyrim share their axes, so Source's view axes are Skyrim world vectors.
		bool HeldCorrection(const GmskyRndFrame& a_f, const GmskyOrigin& a_origin, const RE::NiPoint3& a_cam, const RE::NiMatrix3& a_rot, float a_out[3][4],
			double& a_deg, double& a_units)
		{
			if (!(a_f.flags & GMSKY_RND_FRAME_VIEW)) {
				return false;
			}
			double gf[3], gr[3], gu[3];
			SourceAxes(a_f.view_angles, gf, gr, gu);
			// Skyrim's camera: columns forward, up, right (SkyCraft's RenderPasses).
			const double sf[3] = { a_rot.entry[0][0], a_rot.entry[1][0], a_rot.entry[2][0] };
			const double su[3] = { a_rot.entry[0][1], a_rot.entry[1][1], a_rot.entry[2][1] };
			const double sr[3] = { a_rot.entry[0][2], a_rot.entry[1][2], a_rot.entry[2][2] };
			double m[3][3];
			for (int i = 0; i < 3; ++i) {
				for (int j = 0; j < 3; ++j) {
					m[i][j] = sf[i] * gf[j] + su[i] * gu[j] + sr[i] * gr[j];
				}
			}
			double e[3];
			for (int i = 0; i < 3; ++i) {
				e[i] = a_origin.skyrim_origin[i] + double(a_f.view_eye[i]) / kGmodPerSkyrim - double(i == 0 ? a_cam.x : i == 1 ? a_cam.y : a_cam.z);
			}
			for (int i = 0; i < 3; ++i) {
				for (int j = 0; j < 3; ++j) {
					a_out[i][j] = float(m[i][j]);
				}
				a_out[i][3] = float(-(m[i][0] * e[0] + m[i][1] * e[1] + m[i][2] * e[2]));
			}
			// The largest angle between matching axes (forward, up, right): small while turning, ~180 if
			// an axis convention were mirrored (Skyrim's camera columns are SkyCraft's reading).
			auto angle = [](const double* a, const double* b) {
				return std::acos(std::clamp(a[0] * b[0] + a[1] * b[1] + a[2] * b[2], -1.0, 1.0)) * 180.0 / kPi;
			};
			a_deg = std::max({ angle(sf, gf), angle(su, gu), angle(sr, gr) });
			a_units = std::sqrt(e[0] * e[0] + e[1] * e[1] + e[2] * e[2]);
			return true;
		}

		// Shader resources the passes read besides the model's own.
		struct PassInputs
		{
			bool shadows = false;                             // our shadow map pass + Skyrim's darkening pass
			bool contact = false;                             // contact shadows (Skyrim's darkening pass even without sun shadows)
			ID3D11ShaderResourceView* skyrimShadows = nullptr;  // Skyrim's sun cascades (copied)
			ID3D11ShaderResourceView* sceneDepth = nullptr;   // Skyrim's depth
			ID3D11ShaderResourceView* mask = nullptr;         // Skyrim's shadow mask
			D3D11_VIEWPORT viewport{};
		};

		void DrawModels(ID3D11DeviceContext* a_context, const RE::NiPoint3& a_cam, const RE::NiMatrix3& a_rot, const FrameConstants& a_fc, bool a_reversed,
			ID3D11RenderTargetView* const* a_solidTargets, UINT a_solidCount, ID3D11DepthStencilView* a_dsv, const PassInputs& a_in, Seen& a_seen)
		{
			const auto& frame = Render::NewestFrame();
			a_seen.gmodFrame = frame.valid ? frame.header.frame : 0;
			if (!frame.valid || frame.objects.empty()) {
				g_lastMatrices.clear();
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
				g_lastMatrices.clear();
				return;
			}
			// Motion vectors: each object's matrices as drawn on the last Skyrim frame (its own if new).
			static std::vector<GmskyRndMatrix> prev;
			prev = frame.matrices;
			for (const auto& o : frame.objects) {
				const auto it = g_lastMatrices.find(o.entity);
				if (it != g_lastMatrices.end() && it->second.size() == o.matrix_count) {
					std::copy(it->second.begin(), it->second.end(), prev.begin() + o.first_matrix);
				}
			}
			if (!UploadRows(a_context, g_bones, frame.matrices) || !UploadRows(a_context, g_prevBones, prev)) {
				return;
			}
			g_lastMatrices.clear();
			for (const auto& o : frame.objects) {
				g_lastMatrices[o.entity].assign(frame.matrices.begin() + o.first_matrix, frame.matrices.begin() + o.first_matrix + o.matrix_count);
			}

			float held[3][4]{};
			double heldDeg = 0, heldUnits = 0;
			const bool canHold = HeldCorrection(frame.header, origin, a_cam, a_rot, held, heldDeg, heldUnits);
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
			ID3D11ShaderResourceView* vsSrvs[3] = { g_bones.srv, nullptr, g_prevBones.srv };
			a_context->VSSetShaderResources(0, 3, vsSrvs);
			ID3D11SamplerState* samplers[3] = { g_sampler, g_pointSampler, g_shadowSampler };
			a_context->PSSetSamplers(0, 3, samplers);
			const UINT stride = sizeof(GmskyRndVertex), zero = 0;
			ID3D11ShaderResourceView* noSrvs[8]{};

			// Fills the object's constants for one part (the held correction applies in every pass).
			auto setObject = [&](const GmskyRndObject& o, const GmskyRndPart& part, bool isHeld, bool hasNormal, bool hasExponent = false, bool hasGlowMask = false) {
				ObjectConstants oc{};
				oc.offset[0] = offset[0], oc.offset[1] = offset[1], oc.offset[2] = offset[2], oc.offset[3] = perGmod;
				oc.info[0] = o.first_matrix;
				oc.info[1] = part.flags;
				oc.info[2] = isHeld ? 1u : 0u;
				oc.info[3] = hasNormal ? 1u : 0u;
				for (int c = 0; c < 4; ++c) {
					oc.color[c] = float((o.color >> (8 * c)) & 0xFF) / 255.0f;
				}
				oc.alpha[0] = part.alpha_ref;
				oc.alpha[1] = part.phong_exponent;
				oc.alpha[2] = part.phong_boost;
				oc.alpha[3] = (o.flags & GMSKY_RND_OBJ_CONTACT) ? 1.0f : 0.0f;
				std::memcpy(oc.held, held, sizeof(held));
				oc.fresnel[0] = part.phong_fresnel[0], oc.fresnel[1] = part.phong_fresnel[1], oc.fresnel[2] = part.phong_fresnel[2];
				oc.fresnel[3] = (part.flags & GMSKY_RND_MAT_ALBEDOTINT) ? 1.0f : 0.0f;
				oc.tint[0] = part.phong_tint[0], oc.tint[1] = part.phong_tint[1], oc.tint[2] = part.phong_tint[2];
				oc.tint[3] = hasExponent ? 1.0f : 0.0f;
				oc.selfIllum[0] = part.selfillum_tint[0], oc.selfIllum[1] = part.selfillum_tint[1], oc.selfIllum[2] = part.selfillum_tint[2];
				oc.selfIllum[3] = hasGlowMask ? 1.0f : 0.0f;
				D3D11_MAPPED_SUBRESOURCE mapped{};
				if (FAILED(a_context->Map(g_objectCb, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
					return false;
				}
				std::memcpy(mapped.pData, &oc, sizeof(oc));
				a_context->Unmap(g_objectCb, 0);
				return true;
			};
			auto textureOf = [&](uint32_t a_id) -> ID3D11ShaderResourceView* {
				const auto tex = g_gpuTextures.find(a_id);
				return tex != g_gpuTextures.end() && tex->second.srv ? tex->second.srv : nullptr;
			};

			if (a_in.shadows) {
				// Our objects into our sun shadow map (solid and alpha-tested parts cast; see-through ones don't).
				a_context->PSSetShaderResources(0, 8, noSrvs);
				a_context->OMSetRenderTargets(0, nullptr, g_shadowDsv);
				a_context->ClearDepthStencilView(g_shadowDsv, D3D11_CLEAR_DEPTH, 1.0f, 0);
				D3D11_VIEWPORT svp{ 0, 0, float(kShadowSize), float(kShadowSize), 0, 1 };
				a_context->RSSetViewports(1, &svp);
				a_context->RSSetState(g_shadowRaster);
				a_context->OMSetDepthStencilState(g_depthWrite, 0);
				a_context->VSSetShader(g_shadowVs, nullptr, 0);
				a_context->PSSetShader(g_shadowPs, nullptr, 0);
				for (const auto& o : frame.objects) {
					const auto it = g_gpuModels.find(o.model_id);
					if (it == g_gpuModels.end() || !it->second.vb || o.matrix_count != it->second.bones) {
						continue;
					}
					const auto& m = it->second;
					const bool isHeld = (o.flags & GMSKY_RND_OBJ_HELD) && canHold;
					a_context->IASetVertexBuffers(0, 1, &m.vb, &stride, &zero);
					a_context->IASetIndexBuffer(m.ib, DXGI_FORMAT_R32_UINT, 0);
					for (const auto& part : m.parts) {
						if (part.flags & (GMSKY_RND_MAT_TRANSLUCENT | GMSKY_RND_MAT_ADDITIVE) || !setObject(o, part, isHeld, false)) {
							continue;
						}
						ID3D11ShaderResourceView* srv = textureOf(part.texture_id);
						srv = srv ? srv : g_whiteSrv;
						a_context->PSSetShaderResources(1, 1, &srv);
						a_context->DrawIndexed(part.index_count, part.first_index, 0);
					}
					++a_seen.shadowCasters;
				}
				++a_seen.shadowFrames;
			}
			if ((a_in.shadows || a_in.contact) && a_in.sceneDepth) {
				// Their shadows and the characters' contact shadows on Skyrim: a full-screen multiply pass
				// over Skyrim's colour (no depth bound, so Skyrim's depth can be read).
				{
					a_context->PSSetShaderResources(0, 8, noSrvs);
					a_context->OMSetRenderTargets(1, a_solidTargets, nullptr);
					a_context->RSSetViewports(1, &a_in.viewport);
					a_context->RSSetState(g_raster);
					a_context->OMSetBlendState(g_multiplyBlend, nullptr, 0xFFFFFFFF);
					a_context->OMSetDepthStencilState(nullptr, 0);
					a_context->IASetInputLayout(nullptr);
					a_context->VSSetShader(g_fullVs, nullptr, 0);
					a_context->PSSetShader(g_overlayPs, nullptr, 0);
					ID3D11ShaderResourceView* overlaySrvs[4] = { g_shadowSrv, nullptr, a_in.sceneDepth, a_in.mask };
					a_context->PSSetShaderResources(4, 4, overlaySrvs);
					a_context->Draw(3, 0);
					++a_seen.overlayFrames;
					a_context->PSSetShaderResources(0, 8, noSrvs);
				}
			}
			if (a_in.shadows) {
				ShadowProbe(a_context, a_in.skyrimShadows);
			}
			{
				// Back to the model passes.
				a_context->IASetInputLayout(g_modelLayout);
				a_context->VSSetShader(g_modelVs, nullptr, 0);
				a_context->PSSetShader(g_modelPs, nullptr, 0);
				a_context->RSSetViewports(1, &a_in.viewport);
				a_context->RSSetState(g_raster);
				a_context->OMSetBlendState(g_solidBlend, nullptr, 0xFFFFFFFF);
				a_context->OMSetDepthStencilState(a_reversed ? g_depthWriteRev : g_depthWrite, 0);
			}
			if (a_in.skyrimShadows) {
				++a_seen.skyrimCascadeFrames;
			}

			bool counted = false, heldCounted = false;
			// Pass 0: solid and alpha-tested parts (depth written, motion vectors and normals).
			// Pass 1: see-through ($translucent, blended) and glowing ($additive, added) parts on top,
			// depth tested but not written (SkyCraft's blended pass).
			for (int pass = 0; pass < 2; ++pass) {
				if (pass == 0) {
					a_context->OMSetRenderTargets(a_solidCount, a_solidTargets, a_dsv);
					ID3D11ShaderResourceView* shadowSrvs[2] = { a_in.shadows ? g_shadowSrv : nullptr, a_in.skyrimShadows };
					a_context->PSSetShaderResources(4, 2, shadowSrvs);
				}
				for (const auto& o : frame.objects) {
					const auto it = g_gpuModels.find(o.model_id);
					if (it == g_gpuModels.end() || !it->second.vb) {
						a_seen.skippedUnknown += pass == 0 ? 1 : 0;
						continue;
					}
					const auto& m = it->second;
					if (o.matrix_count != m.bones) {
						a_seen.skippedBones += pass == 0 ? 1 : 0;
						continue;
					}
					const bool isHeld = (o.flags & GMSKY_RND_OBJ_HELD) && canHold;
					if (pass == 0 && (o.flags & GMSKY_RND_OBJ_HELD)) {
						++(canHold ? a_seen.heldDrawn : a_seen.heldNoView);
						if (canHold && !heldCounted) {
							heldCounted = true;
							++a_seen.heldFrames;
							a_seen.heldDegSum += heldDeg;
							a_seen.heldDegMax = std::max(a_seen.heldDegMax, heldDeg);
							a_seen.heldUnitsSum += heldUnits;
							a_seen.heldUnitsMax = std::max(a_seen.heldUnitsMax, heldUnits);
							LARGE_INTEGER q{};
							QueryPerformanceCounter(&q);
							a_seen.heldAgeSum += QpcMs(q.QuadPart - frame.header.qpc);
						}
					}
					a_context->IASetVertexBuffers(0, 1, &m.vb, &stride, &zero);
					a_context->IASetIndexBuffer(m.ib, DXGI_FORMAT_R32_UINT, 0);
					for (const auto& part : m.parts) {
						const bool blended = (part.flags & (GMSKY_RND_MAT_TRANSLUCENT | GMSKY_RND_MAT_ADDITIVE)) != 0;
						if (blended != (pass == 1)) {
							continue;
						}
						ID3D11ShaderResourceView* normalSrv = part.normal_texture_id ? textureOf(part.normal_texture_id) : nullptr;
						ID3D11ShaderResourceView* exponentSrv = part.phong_exponent_texture_id ? textureOf(part.phong_exponent_texture_id) : nullptr;
						ID3D11ShaderResourceView* glowSrv = part.selfillum_mask_texture_id ? textureOf(part.selfillum_mask_texture_id) : nullptr;
						if (!setObject(o, part, isHeld, normalSrv != nullptr, exponentSrv != nullptr, glowSrv != nullptr)) {
							continue;
						}
						a_context->PSSetShaderResources(8, 1, &exponentSrv);
						a_context->PSSetShaderResources(9, 1, &glowSrv);
						a_seen.selfIllumCalls += (part.flags & GMSKY_RND_MAT_SELFILLUM) ? 1 : 0;
						ID3D11ShaderResourceView* srv = textureOf(part.texture_id);
						srv = srv ? srv : g_whiteSrv;
						a_context->PSSetShaderResources(1, 1, &srv);
						a_context->PSSetShaderResources(3, 1, &normalSrv);
						a_seen.normalMapCalls += normalSrv ? 1 : 0;
						a_seen.phongCalls += (part.flags & GMSKY_RND_MAT_PHONG) ? 1 : 0;
						if (pass == 1) {
							a_context->OMSetBlendState((part.flags & GMSKY_RND_MAT_ADDITIVE) ? g_addBlend : g_alphaBlend, nullptr, 0xFFFFFFFF);
							++a_seen.blendedCalls;
						}
						a_context->DrawIndexed(part.index_count, part.first_index, 0);
						++a_seen.drawCalls;
						a_seen.triangles += part.index_count / 3;
					}
					if (pass == 0) {
						++a_seen.objectsDrawn;
					}
					if (pass == 0 && !counted) {
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
				if (pass == 0) {
					// See-through parts: the colour only, depth tested but not written.
					a_context->OMSetRenderTargets(1, a_solidTargets, a_dsv);
					a_context->OMSetDepthStencilState(a_reversed ? g_depthReadRev : g_depthRead, 0);
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
			const auto& normals = rd.renderTargets[RE::RENDER_TARGETS::kNORMAL_TAAMASK_SSRMASK];
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
			static FrameConstants fc;
			fc = FrameConstants{};
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
			const float l[3] = { 0.4f, 0.3f, 0.866f };  // the test cube's fixed light, from the south-east
			fc.lightDir[0] = l[0], fc.lightDir[1] = l[1], fc.lightDir[2] = l[2];
			Set4(fc.camForward, R.entry[0][0], R.entry[1][0], R.entry[2][0], 0.0f);
			Set4(fc.camUp, R.entry[0][1], R.entry[1][1], R.entry[2][1], 0.0f);
			Set4(fc.camRight, R.entry[0][2], R.entry[1][2], R.entry[2][2], 0.0f);

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
			GatherLighting(fc, cam, seen);

			// G5b bundle: Garry's Mod's console switches (frame flags), shadows, Skyrim's depth back to positions.
			const auto& newest = Render::NewestFrame();
			const uint32_t sw = newest.valid ? newest.header.flags : 0;
			seen.switches = sw;
			Set4(fc.switches, (sw & GMSKY_RND_FRAME_FLIP_GREEN) ? 1.0f : 0.0f, (sw & GMSKY_RND_FRAME_NO_NORMALS) ? 0.0f : 1.0f, (sw & GMSKY_RND_FRAME_NO_PHONG) ? 0.0f : 1.0f,
				(sw & GMSKY_RND_FRAME_LINEAR_ALBEDO) ? 1.0f : 0.0f);
			fc.aoParams[1] = (sw & GMSKY_RND_FRAME_SHADOW_DEBUG) ? 1.0f : 0.0f;
			D3D11_TEXTURE2D_DESC sceneDesc{};
			sceneTex->GetDesc(&sceneDesc);
			Set4(fc.screen, 1.0f / float(sceneDesc.Width), 1.0f / float(sceneDesc.Height), 0.0f, 0.0f);
			const auto& frustum = camera->GetRuntimeData2().viewFrustum;
			Set4(fc.depthParams, frustum.fNear, frustum.fFar, reversed ? 1.0f : 0.0f, 0.0f);
			PassInputs in;
			in.sceneDepth = reinterpret_cast<ID3D11ShaderResourceView*>(depth.depthSRV);
			in.mask = reinterpret_cast<ID3D11ShaderResourceView*>(rd.renderTargets[RE::RENDER_TARGETS::kSHADOW_MASK].SRV);
			in.viewport = D3D11_VIEWPORT{ 0, 0, float(sceneDesc.Width), float(sceneDesc.Height), 0, 1 };
			const bool wantShadows = fc.shadowParams[0] > 0.5f && !(sw & GMSKY_RND_FRAME_NO_SHADOWS);
			if (sw & GMSKY_RND_FRAME_NO_SHADOWS) {
				seen.shadowWhy = "gmsky_render_shadows 0";
			}
			const bool canUnproject = Invert(fc.viewProj, fc.invViewProj);
			in.shadows = wantShadows && EnsureShadowMap() && canUnproject;
			GatherContactShadows(fc, cam, seen);
			in.contact = fc.aoParams[0] > 0.0f && canUnproject;
			if (in.shadows) {
				SetSunShadowMatrix(fc, cam);
				in.skyrimShadows = SetSkyrimShadows(fc, cam);
				fc.shadowExtra[0] = (g_maskUsable && in.mask) ? 1.0f : 0.0f;
			} else {
				fc.shadowParams[0] = 0.0f;
				fc.skyShadowSplits[3] = 0.0f;
			}
			seen.maskUsable = g_maskUsable;
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
			if (fc.shadowParams[0] > 0.5f) {
				MaskProbe(context, in.mask);
			}
			D3D11_MAPPED_SUBRESOURCE mapped{};
			if (SUCCEEDED(context->Map(g_frameCb, 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped))) {
				std::memcpy(mapped.pData, &fc, sizeof(fc));
				context->Unmap(g_frameCb, 0);
			}
			D3D11_TEXTURE2D_DESC sd{};
			sceneTex->GetDesc(&sd);
			D3D11_VIEWPORT vp{ 0, 0, float(sd.Width), float(sd.Height), 0, 1 };
			// Unbind the shader resources first: Skyrim may still have its depth or normals bound for reading.
			ID3D11ShaderResourceView* none[StateBackup::kSlots]{};
			context->VSSetShaderResources(0, StateBackup::kSlots, none);
			context->PSSetShaderResources(0, StateBackup::kSlots, none);
			context->CSSetShaderResources(0, StateBackup::kSlots, none);
			// Solid surfaces also write Skyrim's motion vectors and normals (SkyCraft's targets).
			auto* motionRtv = motion.RTV && motion.texture ? reinterpret_cast<ID3D11RenderTargetView*>(motion.RTV) : nullptr;
			auto* normalsRtv = normals.RTV && normals.texture ? reinterpret_cast<ID3D11RenderTargetView*>(normals.RTV) : nullptr;
			ID3D11RenderTargetView* targets[3] = { rtv, motionRtv, normalsRtv };
			const UINT targetCount = normalsRtv && motionRtv ? 3 : motionRtv ? 2 : 1;
			context->OMSetRenderTargets(targetCount, targets, dsv);
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
			DrawModels(context, cam, R, fc, reversed, targets, targetCount, dsv, in, seen);
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

		// ---- I1: Skyrim's indoor shadow lights, logged only (DESIGN.md "indoor shadow probe") ----
		// Each light is logged in full the first time it is drawn (and again every 30 s), then counted.
		std::mutex g_probeLock;
		std::unordered_map<const void*, int64_t> g_probeSeen;  // light -> when last logged in full
		uint32_t g_probeFrustum = 0, g_probeParabolic = 0;      // Render calls since the last LogSecond

		void ProbeShadowLight(RE::BSShadowLight* a_light, const char* a_kind)
		{
			auto* renderer = RE::BSGraphics::Renderer::GetSingleton();
			if (!a_light || !renderer) {
				return;
			}
			{
				std::lock_guard lock(g_probeLock);
				(a_kind[0] == 'f' ? g_probeFrustum : g_probeParabolic) += 1;
				auto& last = g_probeSeen[a_light];
				if (last && QpcMs(Qpc() - last) < 30000.0) {
					return;
				}
				last = Qpc();
			}
			auto* context = reinterpret_cast<ID3D11DeviceContext*>(renderer->GetRuntimeData().context);
			std::string bound = "none";
			ID3D11DepthStencilView* dsv = nullptr;
			context->OMGetRenderTargets(0, nullptr, &dsv);
			if (dsv) {
				ID3D11Resource* res = nullptr;
				dsv->GetResource(&res);
				D3D11_DEPTH_STENCIL_VIEW_DESC vd{};
				dsv->GetDesc(&vd);
				if (res) {
					D3D11_TEXTURE2D_DESC td{};
					static_cast<ID3D11Texture2D*>(res)->GetDesc(&td);
					bound = std::format("{}x{} x{} format {} (view format {}, dimension {}, first slice {}, slices {})", td.Width, td.Height, td.ArraySize, int(td.Format),
						int(vd.Format), int(vd.ViewDimension), vd.Texture2DArray.FirstArraySlice, vd.Texture2DArray.ArraySize);
					Release(res);
				}
				Release(dsv);
			}
			RE::NiPoint3 pos{};
			float radius = 0.0f;
			if (a_light->light) {
				pos = a_light->light->world.translate;
				radius = a_light->light->GetLightRuntimeData().radius.x;
			}
			auto& descs = a_light->GetRuntimeData().shadowmapDescriptors;
			logger::info("shadow light probe: {} light {:p} at ({:.1f}, {:.1f}, {:.1f}) radius {:.0f}; {} descriptors; directional {} frustum {} parabolic {} omni {}; bound depth {}",
				a_kind, static_cast<const void*>(a_light), pos.x, pos.y, pos.z, radius, descs.size(), a_light->GetIsDirectionalLight(), a_light->GetIsFrustumLight(),
				a_light->GetIsParabolicLight(), a_light->GetIsOmniLight(), bound);
			for (std::uint32_t k = 0; k < std::min<std::uint32_t>(descs.size(), 6); ++k) {
				const auto& d = descs[k];
				const auto& m = d.lightTransform.m;
				const auto* port = reinterpret_cast<const std::int32_t*>(&d.port);
				// Where the transform puts the light and points 100 units along each axis (as v * M and as M * v).
				auto row = [&](const RE::NiPoint3& p) {
					float rv[4], cv[4];
					for (int c = 0; c < 4; ++c) {
						rv[c] = p.x * m[0][c] + p.y * m[1][c] + p.z * m[2][c] + m[3][c];
						cv[c] = m[c][0] * p.x + m[c][1] * p.y + m[c][2] * p.z + m[c][3];
					}
					return std::format("v*M ({:.3f} {:.3f} {:.3f} w {:.3f}) M*v ({:.3f} {:.3f} {:.3f} w {:.3f})", rv[0], rv[1], rv[2], rv[3], cv[0], cv[1], cv[2], cv[3]);
				};
				logger::info("shadow light probe:   descriptor {}: render target {} slice {} port ({} {} {} {}) camera {}; transform rows ({:.4g} {:.4g} {:.4g} {:.4g}) ({:.4g} {:.4g} {:.4g} {:.4g}) ({:.4g} {:.4g} {:.4g} {:.4g}) ({:.4g} {:.4g} {:.4g} {:.4g})",
					k, int(d.renderTarget), d.shadowmapIndex, port[0], port[1], port[2], port[3], d.camera ? "yes" : "none", m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1],
					m[1][2], m[1][3], m[2][0], m[2][1], m[2][2], m[2][3], m[3][0], m[3][1], m[3][2], m[3][3]);
				logger::info("shadow light probe:     light {}; +x100 {}; +y100 {}; -z100 {}", row(pos), row(pos + RE::NiPoint3(100, 0, 0)), row(pos + RE::NiPoint3(0, 100, 0)),
					row(pos + RE::NiPoint3(0, 0, -100)));
				if (d.camera) {
					const auto& t = d.camera->world.translate;
					const auto& r = d.camera->world.rotate;
					logger::info("shadow light probe:     camera at ({:.1f}, {:.1f}, {:.1f}) forward ({:.3f} {:.3f} {:.3f}) up ({:.3f} {:.3f} {:.3f}); frustum l {:.3f} r {:.3f} t {:.3f} b {:.3f} near {:.1f} far {:.1f} ortho {}",
						t.x, t.y, t.z, r.entry[0][0], r.entry[1][0], r.entry[2][0], r.entry[0][1], r.entry[1][1], r.entry[2][1], d.camera->GetRuntimeData2().viewFrustum.fLeft,
						d.camera->GetRuntimeData2().viewFrustum.fRight, d.camera->GetRuntimeData2().viewFrustum.fTop, d.camera->GetRuntimeData2().viewFrustum.fBottom,
						d.camera->GetRuntimeData2().viewFrustum.fNear, d.camera->GetRuntimeData2().viewFrustum.fFar, d.camera->GetRuntimeData2().viewFrustum.bOrtho);
				}
			}
		}

		struct FrustumShadowRenderHook
		{
			static void thunk(RE::BSShadowLight* a_this, std::uint32_t& a_index)
			{
				func(a_this, a_index);
				ProbeShadowLight(a_this, "frustum");
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

		struct ParabolicShadowRenderHook
		{
			static void thunk(RE::BSShadowLight* a_this, std::uint32_t& a_index)
			{
				func(a_this, a_index);
				ProbeShadowLight(a_this, "parabolic");
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

		// BSShadowDirectionalLight::Render: the sun's shadow maps are complete when it returns (SkyCraft's hook).
		struct SunShadowRenderHook
		{
			static void thunk(RE::BSShadowLight* a_this, std::uint32_t& a_index)
			{
				func(a_this, a_index);
				CaptureSunShadows(a_this);
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

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
		REL::Relocation<std::uintptr_t> sunVtbl{ RE::VTABLE_BSShadowDirectionalLight[0] };
		SunShadowRenderHook::func = sunVtbl.write_vfunc(0x0A, SunShadowRenderHook::thunk);
		logger::info("shadows: hooked the sun's shadow map rendering (BSShadowDirectionalLight entry 0x0A)");
		REL::Relocation<std::uintptr_t> frustumVtbl{ RE::VTABLE_BSShadowFrustumLight[0] };
		FrustumShadowRenderHook::func = frustumVtbl.write_vfunc(0x0A, FrustumShadowRenderHook::thunk);
		REL::Relocation<std::uintptr_t> parabolicVtbl{ RE::VTABLE_BSShadowParabolicLight[0] };
		ParabolicShadowRenderHook::func = parabolicVtbl.write_vfunc(0x0A, ParabolicShadowRenderHook::thunk);
		logger::info("shadow light probe: hooked frustum and parabolic shadow lights' Render (entry 0x0A), log only");
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
			g_seen.triangles = g_seen.drawCalls = g_seen.blendedCalls = g_seen.framesDrawn = 0;
			g_seen.modelUploads = g_seen.textureUploads = g_seen.uploadFailures = 0;
			g_seen.uploadMsMax = 0;
			g_seen.heldDrawn = g_seen.heldNoView = g_seen.heldFrames = 0;
			g_seen.heldDegSum = g_seen.heldDegMax = g_seen.heldUnitsSum = g_seen.heldUnitsMax = g_seen.heldAgeSum = 0;
			g_seen.shadowFrames = g_seen.shadowCasters = g_seen.overlayFrames = g_seen.skyrimCascadeFrames = g_seen.normalMapCalls = g_seen.phongCalls = 0;
			g_seen.contactBlobs = g_seen.contactFrames = g_seen.selfIllumCalls = 0;
		}
		const auto captures = g_cascadeCaptures.exchange(0);
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
			logger::info("in-frame drawing: Garry's Mod objects: {} frames drawn, {} objects ({} triangles, {} draw calls, {} blended); {} point lights; skipped: {} epoch (frame {} vs origin {}), {} no origin, {} unknown model, {} bone count; "
						 "on the card: {} models, {} textures; uploads {} models, {} textures, {} failed, max {:.2f} ms; Garry's Mod frame {}{}",
				seen.framesDrawn, objectsDrawn, seen.triangles, seen.drawCalls, seen.blendedCalls, seen.pointLights, seen.skippedEpoch, seen.frameEpoch, seen.originEpoch,
				seen.skippedNoOrigin, seen.skippedUnknown, seen.skippedBones, seen.gpuModels, seen.gpuTextures, seen.modelUploads, seen.textureUploads, seen.uploadFailures,
				seen.uploadMsMax, seen.gmodFrame, first);
		}
		if (seen.gmodFrame) {
			std::string sw;
			if (seen.switches & GMSKY_RND_FRAME_FLIP_GREEN) sw += " normal_flip";
			if (seen.switches & GMSKY_RND_FRAME_NO_NORMALS) sw += " no_normals";
			if (seen.switches & GMSKY_RND_FRAME_NO_PHONG) sw += " no_phong";
			if (seen.switches & GMSKY_RND_FRAME_NO_SHADOWS) sw += " no_shadows";
			if (seen.switches & GMSKY_RND_FRAME_LINEAR_ALBEDO) sw += " linear_albedo";
			if (seen.switches & GMSKY_RND_FRAME_SHADOW_DEBUG) sw += " shadow_debug";
			logger::info("in-frame drawing: shadows {} frames ({} casters, {} darkening passes on Skyrim){}; Skyrim's sun cascades used {} frames ({} captured); Skyrim's shadow mask {}; parts with a normal map {}, with phong {}, self-illuminated {}; contact shadows under {:.1f} characters a frame; switches:{}",
				seen.shadowFrames, seen.shadowCasters, seen.overlayFrames, seen.shadowFrames ? "" : std::format(" (off: {})", seen.shadowWhy), seen.skyrimCascadeFrames,
				captures, seen.maskUsable ? "used" : "not used", seen.normalMapCalls, seen.phongCalls, seen.selfIllumCalls,
				seen.contactFrames ? double(seen.contactBlobs) / double(seen.contactFrames) : 0.0, sw.empty() ? " none" : sw);
		}
		{
			uint32_t frustum = 0, parabolic = 0;
			{
				std::lock_guard lock(g_probeLock);
				frustum = std::exchange(g_probeFrustum, 0);
				parabolic = std::exchange(g_probeParabolic, 0);
			}
			uint32_t active = 0, shadowActive = 0;
			std::string kinds;
			if (auto* ssn = RE::BSShaderManager::State::GetSingleton().shadowSceneNode[0]) {
				auto& srd = ssn->GetRuntimeData();
				active = srd.activeLights.size();
				shadowActive = srd.activeShadowLights.size();
				for (auto& l : srd.activeShadowLights) {
					if (l) {
						kinds += l->GetIsDirectionalLight() ? " directional" : l->GetIsFrustumLight() ? " frustum" : l->GetIsParabolicLight() ? " parabolic" : l->GetIsOmniLight() ? " omni" : " other";
					}
				}
			}
			if (frustum || parabolic || shadowActive) {
				logger::info("shadow light probe: {} frustum / {} parabolic shadow renders this second; {} active lights, {} shadow lights:{}", frustum, parabolic, active,
					shadowActive, kinds.empty() ? " none" : kinds);
			}
		}
		if (seen.heldDrawn || seen.heldNoView) {
			const double n = double(std::max<uint64_t>(seen.heldFrames, 1));
			logger::info("in-frame drawing: held objects: {} drawn following Skyrim's camera ({} frames), {} without Garry's Mod's view; Garry's Mod's view of their frame vs Skyrim's camera (largest axis angle): {:.2f} deg avg, {:.2f} max, {:.1f} units avg, {:.1f} max; frame {:.1f} ms old avg",
				seen.heldDrawn, seen.heldFrames, seen.heldNoView, seen.heldDegSum / n, seen.heldDegMax, seen.heldUnitsSum / n, seen.heldUnitsMax, seen.heldAgeSum / n);
		}
	}
}
