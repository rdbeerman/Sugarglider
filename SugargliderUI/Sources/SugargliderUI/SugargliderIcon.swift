// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import SwiftUI

/// Sugar glider icon with animated tail that flows in the wind when tapped
struct SugargliderIcon: View {
    let size: CGFloat
    @State private var isAnimating = false
    @State private var tailSwing: Double = 0

    init(size: CGFloat = 64) {
        self.size = size
    }

    var body: some View {
        ZStack {
            // Tail drawn first (behind body)
            SugargliderTailShape()
                .fill(Color.primary)
                .rotationEffect(
                    .degrees(tailSwing),
                    anchor: UnitPoint(x: 0.48, y: 0.62)
                )

            // Body drawn on top (covers tail connection)
            SugargliderBodyShape()
                .fill(Color.primary)
        }
        .frame(width: size, height: size)
        .onTapGesture {
            startTailAnimation()
        }
        .contentShape(Rectangle())
    }

    private func startTailAnimation() {
        guard !isAnimating else { return }
        isAnimating = true
        tailSwing = 0

        // Animate with a repeating back-and-forth swing
        withAnimation(
            .easeInOut(duration: 0.3)
            .repeatForever(autoreverses: true)
        ) {
            tailSwing = 8
        }

        // Stop after a few seconds
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) {
            withAnimation(.spring(response: 0.4, dampingFraction: 0.5)) {
                tailSwing = 0
            }
            isAnimating = false
        }
    }
}

/// Shape for the sugar glider body - includes extra coverage at tail connection
struct SugargliderBodyShape: Shape {
    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / 24.0
        let offsetX = (rect.width - 24 * scale) / 2
        let offsetY = (rect.height - 24 * scale) / 2

        var path = Path()

        // Start at upper left
        path.move(to: pt(1.71, 1.18, scale, offsetX, offsetY))
        path.addLine(to: pt(1.89, 2.14, scale, offsetX, offsetY))
        path.addLine(to: pt(1.00, 2.54, scale, offsetX, offsetY))
        path.addLine(to: pt(1.00, 3.11, scale, offsetX, offsetY))
        path.addLine(to: pt(1.93, 3.21, scale, offsetX, offsetY))
        path.addLine(to: pt(2.00, 4.04, scale, offsetX, offsetY))
        path.addLine(to: pt(3.36, 3.57, scale, offsetX, offsetY))
        path.addLine(to: pt(4.07, 4.18, scale, offsetX, offsetY))
        path.addLine(to: pt(6.50, 8.68, scale, offsetX, offsetY))
        path.addLine(to: pt(6.36, 10.18, scale, offsetX, offsetY))
        path.addLine(to: pt(5.36, 12.14, scale, offsetX, offsetY))
        path.addLine(to: pt(2.43, 14.29, scale, offsetX, offsetY))
        path.addLine(to: pt(2.46, 14.79, scale, offsetX, offsetY))
        path.addLine(to: pt(3.07, 14.86, scale, offsetX, offsetY))
        path.addLine(to: pt(2.82, 15.79, scale, offsetX, offsetY))
        path.addLine(to: pt(3.82, 15.46, scale, offsetX, offsetY))
        path.addLine(to: pt(3.93, 16.04, scale, offsetX, offsetY))
        path.addLine(to: pt(4.43, 16.11, scale, offsetX, offsetY))
        path.addLine(to: pt(5.04, 14.68, scale, offsetX, offsetY))
        path.addLine(to: pt(6.36, 14.00, scale, offsetX, offsetY))

        // Wider body center area to cover tail connection
        path.addLine(to: pt(8.5, 14.0, scale, offsetX, offsetY))
        path.addLine(to: pt(9.43, 13.96, scale, offsetX, offsetY))
        path.addLine(to: pt(10.5, 14.2, scale, offsetX, offsetY))
        path.addLine(to: pt(11.79, 15.14, scale, offsetX, offsetY))
        path.addLine(to: pt(12.5, 14.5, scale, offsetX, offsetY))
        path.addLine(to: pt(13.5, 14.5, scale, offsetX, offsetY))

        // Right lower paw
        path.addLine(to: pt(14.61, 17.32, scale, offsetX, offsetY))
        path.addLine(to: pt(15.29, 19.57, scale, offsetX, offsetY))
        path.addLine(to: pt(16.04, 19.07, scale, offsetX, offsetY))
        path.addLine(to: pt(17.00, 19.75, scale, offsetX, offsetY))
        path.addLine(to: pt(17.04, 18.68, scale, offsetX, offsetY))
        path.addLine(to: pt(17.75, 18.18, scale, offsetX, offsetY))
        path.addLine(to: pt(15.71, 16.29, scale, offsetX, offsetY))
        path.addLine(to: pt(15.43, 14.18, scale, offsetX, offsetY))
        path.addLine(to: pt(15.93, 12.71, scale, offsetX, offsetY))
        path.addLine(to: pt(19.29, 10.00, scale, offsetX, offsetY))
        path.addLine(to: pt(20.57, 9.82, scale, offsetX, offsetY))
        path.addLine(to: pt(21.54, 10.86, scale, offsetX, offsetY))
        path.addLine(to: pt(21.96, 10.11, scale, offsetX, offsetY))
        path.addLine(to: pt(22.82, 10.25, scale, offsetX, offsetY))
        path.addLine(to: pt(22.96, 9.71, scale, offsetX, offsetY))
        path.addLine(to: pt(22.39, 9.18, scale, offsetX, offsetY))
        path.addLine(to: pt(22.86, 8.50, scale, offsetX, offsetY))
        path.addLine(to: pt(18.89, 8.32, scale, offsetX, offsetY))
        path.addLine(to: pt(16.21, 7.21, scale, offsetX, offsetY))
        path.addLine(to: pt(18.00, 4.57, scale, offsetX, offsetY))
        path.addLine(to: pt(17.82, 3.39, scale, offsetX, offsetY))
        path.addLine(to: pt(15.64, 3.86, scale, offsetX, offsetY))
        path.addLine(to: pt(12.96, 3.14, scale, offsetX, offsetY))
        path.addLine(to: pt(11.79, 1.68, scale, offsetX, offsetY))
        path.addLine(to: pt(10.75, 1.36, scale, offsetX, offsetY))
        path.addLine(to: pt(10.21, 2.50, scale, offsetX, offsetY))
        path.addLine(to: pt(10.68, 4.50, scale, offsetX, offsetY))
        path.addLine(to: pt(9.86, 4.82, scale, offsetX, offsetY))
        path.addLine(to: pt(4.71, 2.86, scale, offsetX, offsetY))
        path.addLine(to: pt(2.64, 1.18, scale, offsetX, offsetY))
        path.closeSubpath()

        return path
    }

    private func pt(_ x: CGFloat, _ y: CGFloat, _ scale: CGFloat, _ offsetX: CGFloat, _ offsetY: CGFloat) -> CGPoint {
        CGPoint(x: x * scale + offsetX, y: y * scale + offsetY)
    }
}

/// Shape for the sugar glider tail with smooth curves
struct SugargliderTailShape: Shape {
    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / 24.0
        let offsetX = (rect.width - 24 * scale) / 2
        let offsetY = (rect.height - 24 * scale) / 2

        var path = Path()

        // Start deeper in the body so it's covered
        path.move(to: pt(10.5, 13.5, scale, offsetX, offsetY))

        // Curve into tail
        path.addQuadCurve(
            to: pt(8.64, 15.93, scale, offsetX, offsetY),
            control: pt(9.5, 14.5, scale, offsetX, offsetY)
        )

        path.addQuadCurve(
            to: pt(7.04, 17.79, scale, offsetX, offsetY),
            control: pt(7.8, 16.8, scale, offsetX, offsetY)
        )

        // Curve to tail tip
        path.addQuadCurve(
            to: pt(1.79, 20.46, scale, offsetX, offsetY),
            control: pt(4.0, 19.0, scale, offsetX, offsetY)
        )

        path.addQuadCurve(
            to: pt(1.43, 21.25, scale, offsetX, offsetY),
            control: pt(1.5, 20.8, scale, offsetX, offsetY)
        )

        // Round tail tip
        path.addQuadCurve(
            to: pt(1.75, 22.25, scale, offsetX, offsetY),
            control: pt(1.2, 21.8, scale, offsetX, offsetY)
        )

        path.addQuadCurve(
            to: pt(4.14, 22.79, scale, offsetX, offsetY),
            control: pt(2.8, 22.7, scale, offsetX, offsetY)
        )

        // Curve back
        path.addQuadCurve(
            to: pt(7.36, 21.50, scale, offsetX, offsetY),
            control: pt(5.8, 22.3, scale, offsetX, offsetY)
        )

        path.addQuadCurve(
            to: pt(9.93, 18.93, scale, offsetX, offsetY),
            control: pt(8.8, 20.3, scale, offsetX, offsetY)
        )

        path.addQuadCurve(
            to: pt(11.79, 15.14, scale, offsetX, offsetY),
            control: pt(11.0, 17.0, scale, offsetX, offsetY)
        )

        // Back into body
        path.addQuadCurve(
            to: pt(10.5, 13.5, scale, offsetX, offsetY),
            control: pt(11.5, 14.0, scale, offsetX, offsetY)
        )

        path.closeSubpath()

        return path
    }

    private func pt(_ x: CGFloat, _ y: CGFloat, _ scale: CGFloat, _ offsetX: CGFloat, _ offsetY: CGFloat) -> CGPoint {
        CGPoint(x: x * scale + offsetX, y: y * scale + offsetY)
    }
}

#Preview {
    VStack(spacing: 32) {
        SugargliderIcon(size: 128)

        Text("Tap the sugar glider!")
            .font(.caption)
            .foregroundColor(.secondary)
    }
    .padding(40)
    .frame(width: 300, height: 300)
}
