import AppKit
import Foundation

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
  fputs("Usage: generate_dmg_background.swift <output-path>\n", stderr)
  exit(1)
}

let outputPath = arguments[1]
let canvasSize = NSSize(width: 660, height: 400)
let canvasRect = NSRect(origin: .zero, size: canvasSize)

guard let bitmap = NSBitmapImageRep(
  bitmapDataPlanes: nil,
  pixelsWide: Int(canvasSize.width),
  pixelsHigh: Int(canvasSize.height),
  bitsPerSample: 8,
  samplesPerPixel: 4,
  hasAlpha: true,
  isPlanar: false,
  colorSpaceName: .deviceRGB,
  bytesPerRow: 0,
  bitsPerPixel: 0
) else {
  fputs("Failed to create bitmap canvas.\n", stderr)
  exit(1)
}

guard let graphicsContext = NSGraphicsContext(bitmapImageRep: bitmap) else {
  fputs("Failed to initialize graphics context.\n", stderr)
  exit(1)
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = graphicsContext

let gradient = NSGradient(
  colors: [
    NSColor(red: 0.04, green: 0.17, blue: 0.38, alpha: 1.0),
    NSColor(red: 0.12, green: 0.36, blue: 0.70, alpha: 1.0),
  ]
)!
gradient.draw(in: canvasRect, angle: 0)

let appIconCenterX: CGFloat = 180
let applicationsIconCenterX: CGFloat = 480
let iconGuideCenterY: CGFloat = 178

func drawDropPanel(centerX: CGFloat) {
  let panelRect = NSRect(
    x: centerX - 102,
    y: iconGuideCenterY - 96,
    width: 204,
    height: 192
  )
  let panel = NSBezierPath(roundedRect: panelRect, xRadius: 22, yRadius: 22)
  NSColor(calibratedWhite: 1.0, alpha: 0.17).setFill()
  panel.fill()

  NSColor(calibratedWhite: 1.0, alpha: 0.12).setStroke()
  panel.lineWidth = 1.5
  panel.stroke()
}

drawDropPanel(centerX: appIconCenterX)
drawDropPanel(centerX: applicationsIconCenterX)

let titleAttributes: [NSAttributedString.Key: Any] = [
  .font: NSFont.systemFont(ofSize: 33, weight: .bold),
  .foregroundColor: NSColor.white,
]
let title = "Drag ClawPal to Applications"
let titleRect = NSRect(x: 58, y: 314, width: 548, height: 44)
title.draw(in: titleRect, withAttributes: titleAttributes)

let subtitleAttributes: [NSAttributedString.Key: Any] = [
  .font: NSFont.systemFont(ofSize: 16, weight: .medium),
  .foregroundColor: NSColor(calibratedWhite: 1.0, alpha: 0.9),
]
let subtitle = "Drop ClawPal.app into Applications to install"
let subtitleRect = NSRect(x: 58, y: 286, width: 548, height: 24)
subtitle.draw(in: subtitleRect, withAttributes: subtitleAttributes)

let arrowShiftX: CGFloat = -20
let arrowHeadScale: CGFloat = 0.6
let arrowBaseX: CGFloat = 380 + arrowShiftX
let arrowTipX: CGFloat = arrowBaseX + (66 * arrowHeadScale)
let arrowHalfHeight: CGFloat = 35 * arrowHeadScale

let arrowPath = NSBezierPath()
arrowPath.lineWidth = 12
arrowPath.lineCapStyle = .round
arrowPath.move(to: NSPoint(x: 286 + arrowShiftX, y: iconGuideCenterY))
arrowPath.line(to: NSPoint(x: arrowBaseX, y: iconGuideCenterY))
NSColor(calibratedWhite: 1.0, alpha: 0.92).setStroke()
arrowPath.stroke()

let arrowHead = NSBezierPath()
arrowHead.move(to: NSPoint(x: arrowBaseX, y: iconGuideCenterY + arrowHalfHeight))
arrowHead.line(to: NSPoint(x: arrowTipX, y: iconGuideCenterY))
arrowHead.line(to: NSPoint(x: arrowBaseX, y: iconGuideCenterY - arrowHalfHeight))
arrowHead.close()
NSColor(calibratedWhite: 1.0, alpha: 0.92).setFill()
arrowHead.fill()

NSGraphicsContext.restoreGraphicsState()

guard let pngData = bitmap.representation(using: .png, properties: [:]) else {
  fputs("Failed to encode PNG data.\n", stderr)
  exit(1)
}

let outputURL = URL(fileURLWithPath: outputPath)
do {
  try FileManager.default.createDirectory(
    at: outputURL.deletingLastPathComponent(),
    withIntermediateDirectories: true,
    attributes: nil
  )
  try pngData.write(to: outputURL, options: .atomic)
  print("Wrote DMG background to \(outputURL.path)")
} catch {
  fputs("Failed to write image: \(error)\n", stderr)
  exit(1)
}
