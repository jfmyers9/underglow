#!/usr/bin/swift
// Artwork preparation only; normal builds use the checked-in assets.
import Foundation
import CoreGraphics
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard CommandLine.arguments.count == 3 else {
    fail("usage: swift scripts/prepare-icon.swift SOURCE.png OUTPUT_DIRECTORY")
}
let input = URL(fileURLWithPath: CommandLine.arguments[1])
let output = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
guard let source = CGImageSourceCreateWithURL(input as CFURL, nil),
      let original = CGImageSourceCreateImageAtIndex(source, 0, nil),
      original.width == original.height else {
    fail("Expected the square Neon RGB Mechanical Keycap Icon PNG")
}

let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
func context(_ size: Int) -> CGContext {
    guard let result = CGContext(data: nil, width: size, height: size,
                                 bitsPerComponent: 8, bytesPerRow: size * 4,
                                 space: colorSpace,
                                 bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        fail("Cannot allocate icon bitmap")
    }
    result.interpolationQuality = .high
    return result
}

// Trace this artwork's outer tile, not its dark pixels: a color-key removal
// would punch holes through the keycap and its shadows. Coordinates are in a
// 256px, top-left-origin reference; retain the bevel and antialias the edge.
let size = 1024
func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
    CGPoint(x: x * 4, y: (256 - y) * 4)
}
let outline = CGMutablePath()
outline.move(to: point(80, 23))
outline.addLine(to: point(176, 23))
outline.addCurve(to: point(230.5, 80), control1: point(217, 23), control2: point(230.5, 36))
outline.addLine(to: point(230.5, 176))
outline.addCurve(to: point(176, 229), control1: point(230.5, 216), control2: point(216, 229))
outline.addLine(to: point(80, 229))
outline.addCurve(to: point(22.5, 176), control1: point(38, 229), control2: point(22.5, 216))
outline.addLine(to: point(22.5, 80))
outline.addCurve(to: point(80, 23), control1: point(22.5, 36), control2: point(36, 23))
outline.closeSubpath()
let canvas = context(size)
canvas.addPath(outline)
canvas.clip()
canvas.draw(original, in: CGRect(x: 0, y: 0, width: size, height: size))
let master = canvas.makeImage()!

func writePNG(_ image: CGImage, _ url: URL) {
    guard let destination = CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil) else {
        fail("Cannot write \(url.path)")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { fail("PNG encoding failed") }
}

let files = FileManager.default
try files.createDirectory(at: output, withIntermediateDirectories: true)
writePNG(master, output.appendingPathComponent("underglow.png"))
let temporary = files.temporaryDirectory.appendingPathComponent("underglow-icon-\(UUID().uuidString)")
let iconset = temporary.appendingPathComponent("Underglow.iconset")
try files.createDirectory(at: iconset, withIntermediateDirectories: true)
defer { try? files.removeItem(at: temporary) }
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = base * scale
        let resized = context(pixels)
        resized.draw(master, in: CGRect(x: 0, y: 0, width: pixels, height: pixels))
        let image = resized.makeImage()!
        let suffix = scale == 2 ? "@2x" : ""
        writePNG(image, iconset.appendingPathComponent("icon_\(base)x\(base)\(suffix).png"))
        if base == 256 && scale == 1 {
            writePNG(image, output.appendingPathComponent("underglow-256.png"))
        }
    }
}
let converter = Process()
converter.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
converter.arguments = ["-c", "icns", iconset.path, "-o", output.appendingPathComponent("Underglow.icns").path]
try converter.run()
converter.waitUntilExit()
guard converter.terminationStatus == 0 else { fail("iconutil failed") }
print("Prepared transparent PNGs and macOS ICNS in \(output.path)")
