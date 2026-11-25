// Draws the menu bar's Yoru, app/src-tauri/icons/tray-template.png, from the
// shapes of app/src-tauri/icons/tray-template.svg: black on clear, since
// macOS tints a template image itself. Run: swift tools/tray_icon.swift
import AppKit

let box = NSRect(x: 112, y: 272, width: 800, height: 552)  // the SVG's viewBox
let height = 72.0  // pixels; shown 18 points tall, so sharp at 2x and 4x
let scale = height / box.height
let size = NSSize(width: (box.width * scale).rounded(), height: height)

let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width), pixelsHigh: Int(size.height), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let cg = NSGraphicsContext.current!.cgContext
// SVG's y grows downwards.
cg.translateBy(x: 0, y: size.height)
cg.scaleBy(x: scale, y: -scale)
cg.translateBy(x: -box.minX, y: -box.minY)

func ellipse(_ cx: CGFloat, _ cy: CGFloat, _ rx: CGFloat, _ ry: CGFloat, _ degrees: CGFloat) -> CGPath {
    var t = CGAffineTransform(translationX: cx, y: cy).rotated(by: degrees * .pi / 180)
    return CGPath(ellipseIn: CGRect(x: -rx, y: -ry, width: 2 * rx, height: 2 * ry), transform: &t)
}
let body = CGMutablePath()
body.addPath(ellipse(186, 618, 58, 46, -12))
body.addPath(ellipse(842, 438, 58, 46, -28))
body.addEllipse(in: CGRect(x: 512 - 262, y: 548 - 262, width: 524, height: 524))
cg.setFillColor(NSColor.black.cgColor)
cg.addPath(body)
cg.fillPath()
// The eyes go through.
cg.setBlendMode(.clear)
for x in [434.0, 542.0] {
    cg.addPath(CGPath(roundedRect: CGRect(x: x, y: 520, width: 48, height: 108), cornerWidth: 24, cornerHeight: 24, transform: nil))
}
cg.fillPath()
NSGraphicsContext.restoreGraphicsState()

let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "app/src-tauri/icons/tray-template.png")
try! rep.representation(using: .png, properties: [:])!.write(to: out)
print("wrote \(out.path) (\(Int(size.width))x\(Int(size.height)))")
