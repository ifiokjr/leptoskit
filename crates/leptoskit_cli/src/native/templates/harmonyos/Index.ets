import { NativeRequest, Runtime, RenderBatch, RiveFrame } from 'libleptoskit.so';
import { NativePlugin, NativePlugins } from '../leptoskit/plugins';
import { nativePlugins } from '../leptoskit/registrant';
import { Widget, Scene, color } from '../scene';
import { displaySync } from '@kit.ArkGraphics2D';

class RiveTarget {
  context: CanvasRenderingContext2D;
  width: number;
  height: number;
  frame: RiveFrame | undefined;
  constructor(context: CanvasRenderingContext2D, width: number, height: number, frame: RiveFrame | undefined) {
    this.context = context;
    this.width = width;
    this.height = height;
    this.frame = frame;
  }
  draw(): void {
    if (this.frame === undefined) { return; }
    this.context.clearRect(0, 0, this.width, this.height);
    this.context.drawImage(this.frame.pixelMap, 0, 0, this.width, this.height);
  }
}

@Component
struct RiveCanvas {
  @Prop node: Widget = new Widget(0, 'rive', {});
  register: (id: number, context: CanvasRenderingContext2D | undefined) => void = () => {};
  visible: (id: number, shown: boolean) => void = () => {};
  private context: CanvasRenderingContext2D = new CanvasRenderingContext2D(new RenderingContextSettings(true));

  aboutToDisappear(): void {
    this.register(this.node.id, undefined);
    this.visible(this.node.id, false);
  }

  build() {
    Canvas(this.context).width('100%').height('100%')
      .onReady(() => this.register(this.node.id, this.context))
      .onAreaChange(() => this.register(this.node.id, this.context))
      .onVisibleAreaChange([0], (shown: boolean, ratio: number) => this.visible(this.node.id, shown && ratio > 0))
      .accessibilityLevel('no')
  }
}

@Component
struct NativeWidget {
  @Prop node: Widget = new Widget(0, 'view', {});
  onEvent: (id: number, kind: number, text: string) => void = () => {};
  registerRive: (id: number, context: CanvasRenderingContext2D | undefined) => void = () => {};
  riveVisible: (id: number, shown: boolean) => void = () => {};
  wakeRive: () => void = () => {};

  @Builder
  children() {
    ForEach(this.node.children, (child: Widget) => {
      NativeWidget({ node: child, onEvent: this.onEvent, registerRive: this.registerRive, riveVisible: this.riveVisible, wakeRive: this.wakeRive })
    }, (child: Widget) => child.id.toString())
  }

  textAlign(): TextAlign {
    if (this.node.props.text_align === 'center') {
      return TextAlign.Center;
    }
    return this.node.props.text_align === 'end' ? TextAlign.End : TextAlign.Start;
  }

  fontWeight(): FontWeight {
    if (this.node.props.font_weight === 'bold') {
      return FontWeight.Bold;
    }
    if (this.node.props.font_weight === 'semibold') {
      return FontWeight.Medium;
    }
    return this.node.props.font_weight === 'medium' ? FontWeight.Medium : FontWeight.Normal;
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      if (this.node.kind === 'text') {
        Text(this.node.props.text ?? '')
          .width('100%').height('100%')
          .fontSize(this.node.props.font_size ?? 14)
          .fontWeight(this.fontWeight()).textAlign(this.textAlign())
          .fontColor(color(this.node.props.color, 0xff111827))
      } else if (this.node.kind === 'button') {
        Button(this.node.props.text ?? '', { type: ButtonType.Normal })
          .width('100%').height('100%').borderRadius(0).padding(0)
          .backgroundColor(color(this.node.props.background, 0x00000000))
          .fontColor(color(this.node.props.color, 0xff007dff))
          .fontSize(this.node.props.font_size ?? 14).fontWeight(this.fontWeight())
          .onClick(() => this.onEvent(this.node.id, 0, ''))
      } else if (this.node.kind === 'text_input' || this.node.kind === 'password_input') {
        TextInput({ text: this.node.props.value ?? this.node.props.text ?? '', placeholder: this.node.props.placeholder ?? '' })
          .type(this.node.kind === 'password_input' ? InputType.Password : InputType.Normal)
          .width('100%').height('100%').padding(0).borderRadius(0)
          .backgroundColor(Color.Transparent)
          .fontSize(this.node.props.font_size ?? 14).fontWeight(this.fontWeight())
          .fontColor(color(this.node.props.color, 0xff111827)).textAlign(this.textAlign())
          .onChange((text: string) => this.onEvent(this.node.id, 1, text))
          .onSubmit(() => this.onEvent(this.node.id, 2, this.node.props.value ?? this.node.props.text ?? ''))
          .onFocus(() => this.onEvent(this.node.id, 3, ''))
          .onBlur(() => this.onEvent(this.node.id, 4, ''))
      } else if (this.node.kind === 'image') {
        Image(this.node.props.source ?? '').width('100%').height('100%').objectFit(ImageFit.Contain)
      } else if (this.node.kind === 'rive') {
        RiveCanvas({ node: this.node, register: this.registerRive, visible: this.riveVisible })
      } else if (this.node.kind === 'scroll_view') {
        Scroll() {
          Stack({ alignContent: Alignment.TopStart }) {
            this.children()
          }.width(this.node.contentWidth()).height(this.node.contentHeight())
        }.width('100%').height('100%').scrollable(ScrollDirection.Vertical).onDidScroll(() => this.wakeRive())
      } else {
        this.children()
      }
    }
    .width(this.node.frame.width).height(this.node.frame.height)
    .position({ x: this.node.frame.x, y: this.node.frame.y })
    .backgroundColor(color(this.node.props.background, 0x00000000))
    .borderRadius(this.node.props.corner_radius ?? 0)
    .borderWidth(this.node.props.border_width ?? 0)
    .borderColor(color(this.node.props.border_color, 0x00000000))
    .opacity(this.node.props.opacity ?? 1)
    .enabled(this.node.props.enabled ?? true)
    .visibility(this.node.props.hidden ? Visibility.Hidden : Visibility.Visible)
    .clip((this.node.props.clip ?? false) || (this.node.props.corner_radius ?? 0) > 0)
    .accessibilityText(this.node.props.accessibility_label ?? this.node.props.text ?? '')
    .accessibilityLevel(this.node.props.aria?.aria_hidden ? 'no-hide-descendants' : 'auto')
  }
}

@Entry
@Component
struct Index {
  @StorageLink('leptoskit.rive.active') @Watch('riveEnvironmentChanged') private riveActive: boolean = true;
  @StorageLink('leptoskit.rive.reduceMotion') @Watch('riveEnvironmentChanged') private riveReducedMotion: boolean = false;
  @State root: Widget | undefined = undefined;
  @State failure: string = '';
  private runtime: Runtime | undefined = undefined;
  private plugins: NativePlugins | undefined = undefined;
  private scene: Scene = new Scene();
  private timer: number = -1;
  private viewportWidth: number = 0;
  private viewportHeight: number = 0;
  private riveTargets: Map<number, RiveTarget> = new Map();
  private riveVisibleIds: Set<number> = new Set();
  private riveDisplay: displaySync.DisplaySync | undefined = undefined;
  private riveRunning: boolean = false;
  private rivePrevious: number = 0;

  riveEnvironmentChanged(): void { this.requestRiveFrame(); }

  registerRive(id: number, context: CanvasRenderingContext2D | undefined): void {
    const node = this.scene.nodes.get(id);
    if (context === undefined || node === undefined || node.kind !== 'rive') {
      this.riveTargets.delete(id);
      this.riveVisibleIds.delete(id);
    } else {
      const target = new RiveTarget(context, node.frame.width, node.frame.height, this.riveTargets.get(id)?.frame);
      this.riveTargets.set(id, target);
      target.draw();
    }
    this.requestRiveFrame();
  }

  setRiveVisible(id: number, shown: boolean): void {
    if (shown && this.scene.nodes.get(id)?.kind === 'rive') {
      this.riveVisibleIds.add(id);
    } else { this.riveVisibleIds.delete(id); }
    this.requestRiveFrame();
  }

  stopRive(): void {
    if (this.riveRunning) { this.riveDisplay?.stop(); }
    this.riveRunning = false;
    this.rivePrevious = 0;
  }

  requestRiveFrame(): void {
    if (!this.riveActive || this.runtime === undefined || this.riveTargets.size === 0) { this.stopRive(); return; }
    if (this.riveRunning) { return; }
    if (this.riveDisplay === undefined) {
      this.riveDisplay = displaySync.create();
      this.riveDisplay.setExpectedFrameRateRange({ min: 30, max: 60, expected: 60 });
      this.riveDisplay.on('frame', (info: displaySync.IntervalInfo) => this.drawRive(info.timestamp));
    }
    this.riveRunning = true;
    this.riveDisplay.start();
  }

  drawRive(timestamp: number): void {
    const runtime = this.runtime;
    if (!this.riveActive || runtime === undefined) { this.stopRive(); return; }
    const elapsed = this.rivePrevious === 0 ? 0 : Math.max(0, timestamp - this.rivePrevious) / 1_000_000_000;
    this.rivePrevious = timestamp;
    try {
      // Visibility can arrive before Canvas.onReady. Keep the initial pose
      // dirty until its canvas can actually retain an uploaded frame.
      const visible = Array.from(this.riveVisibleIds).filter((id: number) => this.riveTargets.has(id));
      const batch = runtime.rive(elapsed, vp2px(1), true, this.riveReducedMotion, visible);
      batch.frames.forEach((frame: RiveFrame) => {
        const target = this.riveTargets.get(frame.id);
        if (target !== undefined) {
          target.frame = frame;
          target.draw();
        }
      });
      if (!batch.pending) { this.stopRive(); }
    } catch (error) { this.fail(error); }
  }

  stop(): void {
    this.stopRive();
    this.riveDisplay?.off('frame');
    this.riveDisplay = undefined;
    this.riveTargets.clear();
    this.riveVisibleIds.clear();
    if (this.timer >= 0) {
      clearTimeout(this.timer);
      this.timer = -1;
    }
    const plugins = this.plugins;
    this.plugins = undefined;
    try {
      plugins?.close();
    } finally {
      this.runtime?.close();
      this.runtime = undefined;
    }
  }

  aboutToDisappear(): void {
    this.stop();
    this.viewportWidth = 0;
    this.viewportHeight = 0;
  }

  fail(error: Error): void {
    this.stop();
    this.root = undefined;
    this.failure = error.message;
  }

  publish(batch: RenderBatch): void {
    this.root = this.scene.apply(batch.render);
    const plugins = this.plugins;
    if (plugins !== undefined) {
      batch.cancelled.forEach((id: number) => plugins.cancel(id));
      batch.requests.forEach((request: NativeRequest) => plugins.start(request.id, request.plugin, request.method, request.payload));
    }
    this.riveTargets.forEach((_target: RiveTarget, id: number) => {
      if (this.scene.nodes.get(id)?.kind !== 'rive') { this.riveTargets.delete(id); }
    });
    this.riveVisibleIds.forEach((id: number) => {
      if (this.scene.nodes.get(id)?.kind !== 'rive') { this.riveVisibleIds.delete(id); }
    });
    this.requestRiveFrame();
    if (batch.pending && this.timer < 0) {
      this.timer = setTimeout(() => {
        this.timer = -1;
        try {
          if (this.runtime !== undefined) {
            this.publish(this.runtime.pump());
          }
        } catch (error) {
          this.fail(error);
        }
      }, 0);
    }
  }

  resize(width: number, height: number): void {
    if (this.failure.length > 0 || width <= 0 || height <= 0 || (width === this.viewportWidth && height === this.viewportHeight)) {
      return;
    }
    this.viewportWidth = width;
    this.viewportHeight = height;
    try {
      if (this.runtime === undefined) {
        const context = this.getUIContext().getHostContext();
        if (context === undefined) {
          throw new Error('Missing ability context');
        }
        const bytes = context.resourceManager.getRawFileContentSync('app.wasm');
        const wasm = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
        this.scene = new Scene();
        const runtime = new Runtime(wasm, context.filesDir, context.cacheDir);
        this.runtime = runtime;
        const plugins = new NativePlugins((id: number, code: number, payload: ArrayBuffer) => this.complete(id, code, payload));
        this.plugins = plugins;
        nativePlugins(context).forEach((plugin: NativePlugin) => plugins.add(runtime.register(plugin.name, plugin.methods), plugin));
        this.publish(runtime.start(width, height));
      } else {
        this.publish(this.runtime.resize(width, height, false));
      }
    } catch (error) {
      this.fail(error);
    }
  }

  complete(id: number, code: number, payload: ArrayBuffer): void {
    try {
      if (this.runtime !== undefined) {
        this.publish(this.runtime.complete(id, code, payload));
      }
    } catch (error) {
      this.fail(error);
    }
  }

  event(id: number, kind: number, text: string): void {
    try {
      if (this.runtime !== undefined) {
        this.publish(this.runtime.event(id, kind, text));
      }
    } catch (error) {
      this.fail(error);
    }
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      if (this.failure.length > 0) {
        Text(this.failure).padding(24).fontColor(Color.Red).accessibilityText(this.failure)
      } else if (this.root !== undefined) {
        NativeWidget({ node: this.root,
          onEvent: (id: number, kind: number, text: string) => this.event(id, kind, text),
          registerRive: (id: number, context: CanvasRenderingContext2D | undefined) => this.registerRive(id, context),
          riveVisible: (id: number, shown: boolean) => this.setRiveVisible(id, shown),
          wakeRive: () => this.requestRiveFrame() })
      }
    }.width('100%').height('100%')
    .onAreaChange((_old: Area, area: Area) => this.resize(Number(area.width), Number(area.height)))
  }
}
