// SPDX-License-Identifier: GPL-3.0-or-later
// Render an SVG to a transparent PNG of the given pixel size with AppKit.
//   swift render.swift in.svg out.png SIZE        (square)
//   swift render.swift in.svg out.png WIDTHxHEIGHT
import AppKit
let a = CommandLine.arguments
let dims = a.count == 4 ? a[3].split(separator: "x").compactMap { Int($0) } : []
guard (dims.count == 1 || dims.count == 2), let img = NSImage(contentsOfFile: a[1]) else {
    FileHandle.standardError.write("usage: render.swift in.svg out.png SIZE|WxH (or SVG not loadable)\n".data(using: .utf8)!); exit(1)
}
let w = dims[0], h = dims.count == 2 ? dims[1] : dims[0]
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4,
                           hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
rep.size = NSSize(width: w, height: h)
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSGraphicsContext.current?.imageInterpolation = .high
NSColor.clear.set()
NSRect(x: 0, y: 0, width: w, height: h).fill()
img.draw(in: NSRect(x: 0, y: 0, width: w, height: h), from: .zero, operation: .sourceOver, fraction: 1)
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: a[2]))
