import SwiftUI

#if canImport(UIKit)
import UIKit
typealias PlatformImage = UIImage
#else
import AppKit
typealias PlatformImage = NSImage
#endif

extension Image {
    init(platform img: PlatformImage) {
        #if canImport(UIKit)
        self.init(uiImage: img)
        #else
        self.init(nsImage: img)
        #endif
    }
}

@MainActor
enum Haptics {
    static func tap() {
        #if os(iOS)
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        #endif
    }
    static func success() {
        #if os(iOS)
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        #endif
    }
    static func warning() {
        #if os(iOS)
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
        #endif
    }
}

/// A JPEG of a picture for upload, at most 2400 px on the long side (site photos from a
/// phone are 12-48 MP; the models read 2-3 MP at most).
@MainActor
func jpegForUpload(_ data: Data) -> Data {
    #if canImport(UIKit)
    guard let img = UIImage(data: data) else { return data }
    let long = max(img.size.width, img.size.height)
    let scale = min(1, 2400 / max(long, 1))
    let size = CGSize(width: (img.size.width * scale).rounded(), height: (img.size.height * scale).rounded())
    let fmt = UIGraphicsImageRendererFormat()
    fmt.scale = 1
    let out = UIGraphicsImageRenderer(size: size, format: fmt).image { _ in img.draw(in: CGRect(origin: .zero, size: size)) }
    return out.jpegData(compressionQuality: 0.85) ?? data
    #else
    return data
    #endif
}

extension View {
    /// iOS-only modifiers, skipped where they do not exist.
    @ViewBuilder func inlineTitle() -> some View {
        #if os(iOS)
        navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }

    @ViewBuilder func urlField() -> some View {
        #if os(iOS)
        keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
        #else
        self
        #endif
    }

    @ViewBuilder func plainField() -> some View {
        #if os(iOS)
        textInputAutocapitalization(.never).autocorrectionDisabled()
        #else
        self
        #endif
    }
}
