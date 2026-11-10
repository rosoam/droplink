import UIKit
import UniformTypeIdentifiers

// Rust callbacks (exported by libdroplink_ios.a).
@_silgen_name("droplink_ios_file_picked")
func rust_picker_picked(_ path: UnsafePointer<CChar>)

@_silgen_name("droplink_ios_file_pick_cancelled")
func rust_picker_cancelled()

/// iOS document picker used by the "Choisir un fichier…" button.
final class DropLinkPicker: NSObject, UIDocumentPickerDelegate {
    static let shared = DropLinkPicker()

    func present() {
        DispatchQueue.main.async {
            let scene = UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .first { $0.activationState == .foregroundActive }
                ?? UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first

            guard let window = scene?.windows.first(where: { $0.isKeyWindow }) ?? scene?.windows.first,
                  let root = window.rootViewController
            else {
                rust_picker_cancelled()
                return
            }

            let picker = UIDocumentPickerViewController(
                forOpeningContentTypes: [UTType.item],
                asCopy: true
            )
            picker.delegate = self
            picker.allowsMultipleSelection = false
            root.present(picker, animated: true)
        }
    }

    func documentPicker(
        _ controller: UIDocumentPickerViewController,
        didPickDocumentsAt urls: [URL]
    ) {
        guard let url = urls.first else {
            rust_picker_cancelled()
            return
        }
        let destination = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(UUID().uuidString)-\(url.lastPathComponent)")
        do {
            try FileManager.default.copyItem(at: url, to: destination)
            destination.path.withCString { rust_picker_picked($0) }
        } catch {
            NSLog("DropLink picker copy failed: \(error)")
            rust_picker_cancelled()
        }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        rust_picker_cancelled()
    }
}

@_cdecl("droplink_ios_pick_file")
public func droplink_ios_pick_file() {
    DropLinkPicker.shared.present()
}
