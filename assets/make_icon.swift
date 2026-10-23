// make_icon.swift —— 把一张满幅方形 PNG 做成 macOS 应用图标：
// 居中缩放到 824，圆角裁切（squircle 近似），放到 1024 透明画布上。
// 用法：swift make_icon.sh 的 swift 部分 <in.png> <out.png>
import AppKit

let args = CommandLine.arguments
guard args.count >= 3, let src = NSImage(contentsOfFile: args[1]) else {
    FileHandle.standardError.write("usage: make_icon <in.png> <out.png>\n".data(using: .utf8)!)
    exit(1)
}
let outPath = args[2]

let canvas = 1024
let inner = 824          // macOS 规范：图标主体占 1024 的 824
let radius: CGFloat = 185

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: canvas, pixelsHigh: canvas,
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
else { exit(1) }

NSGraphicsContext.saveGraphicsState()
let ctx = NSGraphicsContext(bitmapImageRep: rep)!
NSGraphicsContext.current = ctx

let frame = NSRect(x: (canvas - inner) / 2, y: (canvas - inner) / 2,
                   width: inner, height: inner)
NSBezierPath(roundedRect: frame, xRadius: radius, yRadius: radius).addClip()
// 高质量插值
NSGraphicsContext.current?.imageInterpolation = .high
src.draw(in: frame, from: .zero, operation: .sourceOver, fraction: 1.0)

NSGraphicsContext.restoreGraphicsState()
ctx.flushGraphics()

guard let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
try! png.write(to: URL(fileURLWithPath: outPath))
print("wrote \(outPath)")
