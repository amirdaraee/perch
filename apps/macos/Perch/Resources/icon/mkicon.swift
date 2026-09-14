import AppKit

// Perch's app icon, drawn rather than borrowed: SF Symbols may not be used
// as an app icon under Apple's licence, and this needs to be Perch's own mark
// in any case. A bird on a rail — the name, and what the app does: sits above
// your work and watches it.
let S: CGFloat = 1024

func draw(into ctx: CGContext) {
    ctx.setAllowsAntialiasing(true)
    ctx.interpolationQuality = .high

    // macOS content grid: the rounded square is inset from the full canvas.
    let inset: CGFloat = S * 0.098
    let rect = CGRect(x: inset, y: inset, width: S - inset * 2, height: S - inset * 2)
    let radius = rect.width * 0.2237

    let squircle = CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
    ctx.saveGState()
    ctx.addPath(squircle)
    ctx.clip()

    // A deep slate ground, lighter at the top, so the mark reads as lit.
    let space = CGColorSpaceCreateDeviceRGB()
    let grad = CGGradient(colorsSpace: space, colors: [
        CGColor(srgbRed: 0.157, green: 0.180, blue: 0.231, alpha: 1),
        CGColor(srgbRed: 0.078, green: 0.090, blue: 0.122, alpha: 1),
    ] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: 0, y: rect.maxY),
                           end: CGPoint(x: 0, y: rect.minY), options: [])
    ctx.restoreGState()

    let amber = CGColor(srgbRed: 0.925, green: 0.686, blue: 0.290, alpha: 1)
    let cx = S / 2

    // The rail. Perch's whole posture is "above the work, not in it".
    let railW = rect.width * 0.52
    let railH = S * 0.030
    let railY = S * 0.300
    let rail = CGPath(roundedRect: CGRect(x: cx - railW / 2, y: railY, width: railW, height: railH),
                      cornerWidth: railH / 2, cornerHeight: railH / 2, transform: nil)
    ctx.setFillColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.34))
    ctx.addPath(rail)
    ctx.fillPath()

    // Legs.
    ctx.setStrokeColor(amber)
    ctx.setLineWidth(S * 0.020)
    ctx.setLineCap(.round)
    for dx in [-S * 0.052, S * 0.052] {
        ctx.move(to: CGPoint(x: cx + dx, y: railY + railH))
        ctx.addLine(to: CGPoint(x: cx + dx, y: railY + railH + S * 0.070))
    }
    ctx.strokePath()

    // Body: a teardrop leaning back into a raised tail.
    let body = CGMutablePath()
    let bodyBottom = railY + railH + S * 0.068
    body.move(to: CGPoint(x: cx - S * 0.118, y: bodyBottom + S * 0.052))
    body.addCurve(to: CGPoint(x: cx + S * 0.010, y: bodyBottom),
                  control1: CGPoint(x: cx - S * 0.112, y: bodyBottom + S * 0.004),
                  control2: CGPoint(x: cx - S * 0.062, y: bodyBottom - S * 0.006))
    body.addCurve(to: CGPoint(x: cx + S * 0.156, y: bodyBottom + S * 0.150),
                  control1: CGPoint(x: cx + S * 0.098, y: bodyBottom + S * 0.008),
                  control2: CGPoint(x: cx + S * 0.150, y: bodyBottom + S * 0.074))
    // Tail, swept up and back.
    body.addLine(to: CGPoint(x: cx + S * 0.234, y: bodyBottom + S * 0.236))
    body.addLine(to: CGPoint(x: cx + S * 0.124, y: bodyBottom + S * 0.206))
    body.addCurve(to: CGPoint(x: cx - S * 0.118, y: bodyBottom + S * 0.052),
                  control1: CGPoint(x: cx + S * 0.040, y: bodyBottom + S * 0.190),
                  control2: CGPoint(x: cx - S * 0.098, y: bodyBottom + S * 0.140))
    body.closeSubpath()
    ctx.setFillColor(amber)
    ctx.addPath(body)
    ctx.fillPath()

    // Head.
    let headR = S * 0.088
    let headC = CGPoint(x: cx - S * 0.086, y: bodyBottom + S * 0.150)
    ctx.addEllipse(in: CGRect(x: headC.x - headR, y: headC.y - headR, width: headR * 2, height: headR * 2))
    ctx.fillPath()

    // Beak.
    let beak = CGMutablePath()
    beak.move(to: CGPoint(x: headC.x - headR * 0.72, y: headC.y + S * 0.012))
    beak.addLine(to: CGPoint(x: headC.x - headR - S * 0.072, y: headC.y - S * 0.004))
    beak.addLine(to: CGPoint(x: headC.x - headR * 0.72, y: headC.y - S * 0.030))
    beak.closeSubpath()
    ctx.addPath(beak)
    ctx.fillPath()

    // Eye, punched out of the head so it reads at small sizes.
    ctx.setFillColor(CGColor(srgbRed: 0.078, green: 0.090, blue: 0.122, alpha: 1))
    let eyeR = S * 0.021
    ctx.addEllipse(in: CGRect(x: headC.x - S * 0.014 - eyeR, y: headC.y + S * 0.014 - eyeR,
                              width: eyeR * 2, height: eyeR * 2))
    ctx.fillPath()

    // Wing, a darker sweep so the body is not a flat blob.
    let wing = CGMutablePath()
    wing.move(to: CGPoint(x: cx - S * 0.030, y: bodyBottom + S * 0.116))
    wing.addCurve(to: CGPoint(x: cx + S * 0.126, y: bodyBottom + S * 0.166),
                  control1: CGPoint(x: cx + S * 0.044, y: bodyBottom + S * 0.096),
                  control2: CGPoint(x: cx + S * 0.104, y: bodyBottom + S * 0.118))
    wing.addCurve(to: CGPoint(x: cx - S * 0.030, y: bodyBottom + S * 0.116),
                  control1: CGPoint(x: cx + S * 0.060, y: bodyBottom + S * 0.176),
                  control2: CGPoint(x: cx + S * 0.006, y: bodyBottom + S * 0.156))
    wing.closeSubpath()
    ctx.setFillColor(CGColor(srgbRed: 0.78, green: 0.53, blue: 0.18, alpha: 1))
    ctx.addPath(wing)
    ctx.fillPath()
}

let out = CommandLine.arguments[1]
guard let ctx = CGContext(data: nil, width: Int(S), height: Int(S), bitsPerComponent: 8,
                          bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
    fatalError("no context")
}
draw(into: ctx)
let image = ctx.makeImage()!
let rep = NSBitmapImageRep(cgImage: image)
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: out))
print("wrote \(out)")
