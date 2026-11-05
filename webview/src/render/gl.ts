// WebGL2 renderer: instanced rectangles with pixel-snapped 1px strokes, instanced glyph quads,
// and line strips for edges. Geometry is uploaded only when the scene changes; panning and
// zooming only change two uniforms.
import { GlyphAtlas } from "./glyphs.ts";
import type { Frame, Label, Line, RectBatch, Renderer } from "./types.ts";

interface GpuBatch {
  vao: WebGLVertexArrayObject;
  buffers: WebGLBuffer[];
  lastUsed: number;
}

const RECT_VS = `#version 300 es
layout(location=0) in vec2 a_corner;
layout(location=1) in vec4 a_rect;
layout(location=2) in vec4 a_fill;
layout(location=3) in vec4 a_stroke;
layout(location=4) in float a_strokePx;
uniform vec2 u_scale;     // local units -> device px
uniform vec2 u_offset;    // device px
uniform vec2 u_viewport;  // device px
uniform float u_dpr;
out vec2 v_px;
out vec2 v_size;
out vec4 v_fill;
out vec4 v_stroke;
out float v_sw;
void main() {
  vec2 p0 = floor(a_rect.xy * u_scale + u_offset + 0.5);
  vec2 p1 = floor((a_rect.xy + a_rect.zw) * u_scale + u_offset + 0.5);
  p1 = max(p1, p0 + 1.0);
  vec2 size = p1 - p0;
  vec2 px = p0 + a_corner * size;
  v_px = a_corner * size;
  v_size = size;
  v_fill = a_fill;
  v_stroke = a_stroke;
  v_sw = a_strokePx * u_dpr;
  vec2 clip = px / u_viewport * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}`;

const RECT_FS = `#version 300 es
precision highp float;
in vec2 v_px;
in vec2 v_size;
in vec4 v_fill;
in vec4 v_stroke;
in float v_sw;
out vec4 color;
void main() {
  float d = min(min(v_px.x, v_px.y), min(v_size.x - v_px.x, v_size.y - v_px.y));
  vec4 c = (v_sw > 0.0 && d < v_sw) ? v_stroke : v_fill;
  if (c.a <= 0.0) discard;
  color = vec4(c.rgb * c.a, c.a);
}`;

const TEXT_VS = `#version 300 es
layout(location=0) in vec2 a_corner;
layout(location=1) in vec4 a_rect;   // device px
layout(location=2) in vec4 a_uv;     // atlas px
layout(location=3) in vec4 a_color;
uniform vec2 u_viewport;
uniform float u_atlas;
out vec2 v_uv;
out vec4 v_color;
void main() {
  vec2 px = a_rect.xy + a_corner * a_rect.zw;
  v_uv = (a_uv.xy + a_corner * a_uv.zw) / u_atlas;
  v_color = a_color;
  vec2 clip = px / u_viewport * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}`;

const TEXT_FS = `#version 300 es
precision highp float;
uniform sampler2D u_tex;
in vec2 v_uv;
in vec4 v_color;
out vec4 color;
void main() {
  float a = texture(u_tex, v_uv).a * v_color.a;
  if (a <= 0.0) discard;
  color = vec4(v_color.rgb * a, a);
}`;

const LINE_VS = `#version 300 es
layout(location=0) in vec2 a_pos;     // device px
layout(location=1) in float a_dist;   // device px along the polyline
uniform vec2 u_viewport;
out float v_dist;
void main() {
  v_dist = a_dist;
  vec2 clip = a_pos / u_viewport * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}`;

const LINE_FS = `#version 300 es
precision highp float;
uniform vec4 u_color;
uniform float u_style;
uniform float u_dpr;
in float v_dist;
out vec4 color;
void main() {
  float d = v_dist / u_dpr;
  if (u_style > 0.5 && u_style < 1.5 && mod(d, 8.0) > 5.0) discard;   // dashed
  if (u_style > 1.5 && mod(d, 4.0) > 1.5) discard;                    // dotted
  color = vec4(u_color.rgb * u_color.a, u_color.a);
}`;

function compile(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const prog = gl.createProgram()!;
  for (const [type, src] of [
    [gl.VERTEX_SHADER, vs],
    [gl.FRAGMENT_SHADER, fs],
  ] as const) {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`shader: ${gl.getShaderInfoLog(s)}`);
    gl.attachShader(prog, s);
  }
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(`link: ${gl.getProgramInfoLog(prog)}`);
  return prog;
}

export class GLRenderer implements Renderer {
  readonly kind = "webgl2" as const;
  private readonly gl: WebGL2RenderingContext;
  private readonly canvas: HTMLCanvasElement;
  private readonly rectProg: WebGLProgram;
  private readonly textProg: WebGLProgram;
  private readonly lineProg: WebGLProgram;
  private readonly textVao: WebGLVertexArrayObject;
  private readonly lineVao: WebGLVertexArrayObject;
  private readonly corner: WebGLBuffer;
  /** GPU buffers per batch; batches are immutable once built, so identity is the cache key. */
  private readonly gpu = new Map<RectBatch, GpuBatch>();
  private frameNo = 0;
  private readonly textBuf: WebGLBuffer;
  private readonly textColorBuf: WebGLBuffer;
  private readonly lineBuf: WebGLBuffer;
  private readonly tex: WebGLTexture;
  private readonly atlas = new GlyphAtlas();
  private atlasVersion = -1;
  private dpr = 1;
  private wDev = 1;
  private hDev = 1;
  private textData = new Float32Array(0);
  private textColors = new Uint8Array(0);

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const gl = canvas.getContext("webgl2", { antialias: false, premultipliedAlpha: true, alpha: false, preserveDrawingBuffer: false });
    if (!gl) throw new Error("WebGL2 unavailable");
    this.gl = gl;
    this.rectProg = compile(gl, RECT_VS, RECT_FS);
    this.textProg = compile(gl, TEXT_VS, TEXT_FS);
    this.lineProg = compile(gl, LINE_VS, LINE_FS);

    const corner = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, corner);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);

    this.corner = corner;

    // Text VAO.
    this.textVao = gl.createVertexArray()!;
    gl.bindVertexArray(this.textVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, corner);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.textBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.textBuf);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 32, 0);
    gl.vertexAttribDivisor(1, 1);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 4, gl.FLOAT, false, 32, 16);
    gl.vertexAttribDivisor(2, 1);
    this.textColorBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.textColorBuf);
    gl.enableVertexAttribArray(3);
    gl.vertexAttribPointer(3, 4, gl.UNSIGNED_BYTE, true, 4, 0);
    gl.vertexAttribDivisor(3, 1);

    // Line VAO.
    this.lineVao = gl.createVertexArray()!;
    gl.bindVertexArray(this.lineVao);
    this.lineBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.lineBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 12, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 1, gl.FLOAT, false, 12, 8);
    gl.bindVertexArray(null);

    this.tex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  setFont(family: string): void {
    this.atlas.configure(family, this.dpr);
  }

  resize(cssW: number, cssH: number, dpr: number): void {
    this.dpr = dpr;
    this.wDev = Math.max(1, Math.round(cssW * dpr));
    this.hDev = Math.max(1, Math.round(cssH * dpr));
    this.canvas.width = this.wDev;
    this.canvas.height = this.hDev;
    this.canvas.style.width = `${cssW}px`;
    this.canvas.style.height = `${cssH}px`;
    this.atlas.configure(this.atlas.family, dpr);
  }

  private upload(b: RectBatch): GpuBatch {
    let g = this.gpu.get(b);
    if (g) {
      g.lastUsed = this.frameNo;
      return g;
    }
    const gl = this.gl;
    const vao = gl.createVertexArray()!;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.corner);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    const geom = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, geom);
    gl.bufferData(gl.ARRAY_BUFFER, b.geom.subarray(0, b.count * 4), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 16, 0);
    gl.vertexAttribDivisor(1, 1);
    const colors = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, colors);
    gl.bufferData(gl.ARRAY_BUFFER, b.colors.subarray(0, b.count * 8), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 4, gl.UNSIGNED_BYTE, true, 8, 0);
    gl.vertexAttribDivisor(2, 1);
    gl.enableVertexAttribArray(3);
    gl.vertexAttribPointer(3, 4, gl.UNSIGNED_BYTE, true, 8, 4);
    gl.vertexAttribDivisor(3, 1);
    const stroke = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, stroke);
    gl.bufferData(gl.ARRAY_BUFFER, b.stroke.subarray(0, b.count), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(4);
    gl.vertexAttribPointer(4, 1, gl.FLOAT, false, 4, 0);
    gl.vertexAttribDivisor(4, 1);
    gl.bindVertexArray(null);
    g = { vao, buffers: [geom, colors, stroke], lastUsed: this.frameNo };
    this.gpu.set(b, g);
    return g;
  }

  /** Frees GPU buffers of batches not drawn for a while. */
  private evict(): void {
    if (this.gpu.size < 64) return;
    for (const [b, g] of this.gpu) {
      if (this.frameNo - g.lastUsed > 120) {
        this.gl.deleteVertexArray(g.vao);
        for (const buf of g.buffers) this.gl.deleteBuffer(buf);
        this.gpu.delete(b);
      }
    }
  }

  measure(text: string, size: 11 | 12 | 13): number {
    return this.atlas.measure(text, size);
  }

  draw(f: Frame): void {
    const gl = this.gl;
    this.frameNo++;
    const { view } = f;
    const bg = f.background;
    gl.viewport(0, 0, this.wDev, this.hDev);
    gl.disable(gl.SCISSOR_TEST);
    gl.clearColor(bg[0] / 255, bg[1] / 255, bg[2] / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    const dpr = this.dpr;
    const cssW = this.wDev / dpr;
    const cssH = this.hDev / dpr;
    gl.useProgram(this.rectProg);
    const uScale = gl.getUniformLocation(this.rectProg, "u_scale");
    const uOffset = gl.getUniformLocation(this.rectProg, "u_offset");
    gl.uniform2f(gl.getUniformLocation(this.rectProg, "u_viewport"), this.wDev, this.hDev);
    gl.uniform1f(gl.getUniformLocation(this.rectProg, "u_dpr"), dpr);
    const drawBatch = (b: RectBatch) => {
      if (b.count === 0) return;
      const g = this.upload(b);
      const s = view.scale * b.unit * dpr;
      gl.uniform2f(uScale, s, s);
      gl.uniform2f(uOffset, ((b.originX - view.cx) * view.scale + cssW / 2) * dpr, ((b.originY - view.cy) * view.scale + cssH / 2) * dpr);
      gl.bindVertexArray(g.vao);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, b.count);
    };
    gl.enable(gl.SCISSOR_TEST);
    for (const t of f.tiles) {
      // Shared tile edges round to the same pixel, so tiles neither overlap nor leave gaps.
      const x0 = Math.round(((t.clip.x - view.cx) * view.scale + cssW / 2) * dpr);
      const y0 = Math.round(((t.clip.y - view.cy) * view.scale + cssH / 2) * dpr);
      const x1 = Math.round(((t.clip.x + t.clip.w - view.cx) * view.scale + cssW / 2) * dpr);
      const y1 = Math.round(((t.clip.y + t.clip.h - view.cy) * view.scale + cssH / 2) * dpr);
      const cx0 = Math.max(0, x0);
      const cy0 = Math.max(0, y0);
      const cx1 = Math.min(this.wDev, x1);
      const cy1 = Math.min(this.hDev, y1);
      if (cx1 <= cx0 || cy1 <= cy0) continue;
      gl.scissor(cx0, this.hDev - cy1, cx1 - cx0, cy1 - cy0);
      drawBatch(t.batch);
    }
    gl.disable(gl.SCISSOR_TEST);
    if (f.overlay) drawBatch(f.overlay);
    if (f.lines.length) this.drawLines(f.lines);
    if (f.labels.length) this.drawLabels(f.labels);
    gl.bindVertexArray(null);
    this.evict();
  }

  private drawLines(lines: Line[]): void {
    const gl = this.gl;
    const dpr = this.dpr;
    gl.useProgram(this.lineProg);
    gl.uniform2f(gl.getUniformLocation(this.lineProg, "u_viewport"), this.wDev, this.hDev);
    gl.uniform1f(gl.getUniformLocation(this.lineProg, "u_dpr"), dpr);
    gl.bindVertexArray(this.lineVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.lineBuf);
    for (const l of lines) {
      const n = l.points.length / 2;
      if (n < 2) continue;
      // Thick lines as a triangle strip; 1px lines as LINE_STRIP.
      const thick = l.width * dpr > 1.5;
      const data = new Float32Array((thick ? n * 2 : n) * 3);
      let dist = 0;
      for (let i = 0; i < n; i++) {
        const x = l.points[2 * i]! * dpr;
        const y = l.points[2 * i + 1]! * dpr;
        if (i > 0) dist += Math.hypot(x - l.points[2 * i - 2]! * dpr, y - l.points[2 * i - 1]! * dpr);
        if (!thick) {
          data.set([x, y, dist], i * 3);
          continue;
        }
        const j = Math.min(n - 1, i + 1);
        const k = Math.max(0, i - 1);
        let dx = l.points[2 * j]! - l.points[2 * k]!;
        let dy = l.points[2 * j + 1]! - l.points[2 * k + 1]!;
        const len = Math.hypot(dx, dy) || 1;
        dx /= len;
        dy /= len;
        const hw = (l.width * dpr) / 2;
        data.set([x - dy * hw, y + dx * hw, dist, x + dy * hw, y - dx * hw, dist], i * 6);
      }
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STREAM_DRAW);
      gl.uniform4f(gl.getUniformLocation(this.lineProg, "u_color"), l.color[0] / 255, l.color[1] / 255, l.color[2] / 255, l.color[3] / 255);
      gl.uniform1f(gl.getUniformLocation(this.lineProg, "u_style"), l.style);
      gl.drawArrays(thick ? gl.TRIANGLE_STRIP : gl.LINE_STRIP, 0, thick ? n * 2 : n);
    }
  }

  private drawLabels(labels: Label[]): void {
    const gl = this.gl;
    const dpr = this.dpr;
    let glyphs = 0;
    for (const l of labels) glyphs += l.text.length;
    if (this.textData.length < glyphs * 8) {
      this.textData = new Float32Array(glyphs * 8 * 2);
      this.textColors = new Uint8Array(glyphs * 4 * 2);
    }
    let n = 0;
    for (const l of labels) {
      let x = Math.round(l.x * dpr);
      const y = Math.round(l.y * dpr);
      for (const ch of l.text) {
        const g = this.atlas.glyph(ch, l.size);
        if (ch !== " ") {
          this.textData.set([x, y, g.w, g.h, g.u, g.v, g.w, g.h], n * 8);
          this.textColors.set(l.color, n * 4);
          n++;
        }
        x += g.advance * dpr;
      }
    }
    if (this.atlas.version !== this.atlasVersion) {
      gl.bindTexture(gl.TEXTURE_2D, this.tex);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.atlas.canvas as TexImageSource);
      this.atlasVersion = this.atlas.version;
    }
    gl.useProgram(this.textProg);
    gl.uniform2f(gl.getUniformLocation(this.textProg, "u_viewport"), this.wDev, this.hDev);
    gl.uniform1f(gl.getUniformLocation(this.textProg, "u_atlas"), this.atlas.size);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.uniform1i(gl.getUniformLocation(this.textProg, "u_tex"), 0);
    gl.bindVertexArray(this.textVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.textBuf);
    gl.bufferData(gl.ARRAY_BUFFER, this.textData.subarray(0, n * 8), gl.STREAM_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.textColorBuf);
    gl.bufferData(gl.ARRAY_BUFFER, this.textColors.subarray(0, n * 4), gl.STREAM_DRAW);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, n);
  }

  dispose(): void {
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
