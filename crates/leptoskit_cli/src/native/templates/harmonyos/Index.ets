import { Runtime, RenderBatch } from 'libleptoskit.so';
import { Widget, Scene, color } from '../scene';

@Component
struct NativeWidget {
  @Prop node: Widget = new Widget(0, 'view', {});
  onEvent: (id: number, kind: number, text: string) => void = () => {};

  @Builder
  children() {
    ForEach(this.node.children, (child: Widget) => {
      NativeWidget({ node: child, onEvent: this.onEvent })
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
      } else if (this.node.kind === 'scroll_view') {
        Scroll() {
          Stack({ alignContent: Alignment.TopStart }) {
            this.children()
          }.width(this.node.contentWidth()).height(this.node.contentHeight())
        }.width('100%').height('100%').scrollable(ScrollDirection.Vertical)
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
  }
}

@Entry
@Component
struct Index {
  @State root: Widget | undefined = undefined;
  @State failure: string = '';
  private runtime: Runtime | undefined = undefined;
  private scene: Scene = new Scene();
  private timer: number = -1;
  private viewportWidth: number = 0;
  private viewportHeight: number = 0;

  stop(): void {
    if (this.timer >= 0) {
      clearTimeout(this.timer);
      this.timer = -1;
    }
    this.runtime?.close();
    this.runtime = undefined;
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
        this.runtime = new Runtime(wasm, context.filesDir, context.cacheDir);
        this.publish(this.runtime.start(width, height));
      } else {
        this.publish(this.runtime.resize(width, height, false));
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
        NativeWidget({ node: this.root, onEvent: (id: number, kind: number, text: string) => this.event(id, kind, text) })
      }
    }.width('100%').height('100%')
    .onAreaChange((_old: Area, area: Area) => this.resize(Number(area.width), Number(area.height)))
  }
}
