import QuickLook
import UIKit
import UniformTypeIdentifiers
import PhotosUI

// Rust callbacks (exported by libdroplink_ios.a).
@_silgen_name("droplink_ios_file_picked")
func rust_picker_picked(_ path: UnsafePointer<CChar>)

@_silgen_name("droplink_ios_file_pick_cancelled")
func rust_picker_cancelled()

@_silgen_name("droplink_ios_log")
func rust_picker_log(_ message: UnsafePointer<CChar>)

func pickerLog(_ message: String) {
    message.withCString { rust_picker_log($0) }
}

/// iOS media/file chooser used by the "Choisir un fichier…" button.
///
/// Presents an action sheet (Photos & videos / Files) like a share sheet, then hands the chosen
/// item to Rust as a local file path.
final class DropLinkPicker: NSObject, UIDocumentPickerDelegate, PHPickerViewControllerDelegate {
    static let shared = DropLinkPicker()

    func present() {
        DispatchQueue.main.async {
            guard let root = Self.rootViewController() else {
                pickerLog("picker: no root view controller")
                rust_picker_cancelled()
                return
            }
            let sheet = UIAlertController(
                title: "Envoyer",
                message: "Choisir la source du fichier",
                preferredStyle: .actionSheet
            )
            sheet.addAction(UIAlertAction(title: "Photos et vidéos", style: .default) { _ in
                self.presentPhotos(from: root)
            })
            sheet.addAction(UIAlertAction(title: "Fichiers", style: .default) { _ in
                self.presentFiles(from: root)
            })
            sheet.addAction(UIAlertAction(title: "Annuler", style: .cancel) { _ in
                pickerLog("picker cancelled at menu")
                rust_picker_cancelled()
            })
            if let popover = sheet.popoverPresentationController {
                popover.sourceView = root.view
                popover.sourceRect = CGRect(
                    x: root.view.bounds.midX,
                    y: root.view.bounds.maxY - 40,
                    width: 1,
                    height: 1
                )
                popover.permittedArrowDirections = []
            }
            root.present(sheet, animated: true)
            pickerLog("picker menu presented")
        }
    }

    static func rootViewController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first
        let window = scene?.windows.first(where: { $0.isKeyWindow }) ?? scene?.windows.first
        var controller = window?.rootViewController
        while let presented = controller?.presentedViewController {
            controller = presented
        }
        return controller
    }

    private func presentFiles(from root: UIViewController) {
        let picker = UIDocumentPickerViewController(
            forOpeningContentTypes: [UTType.item],
            asCopy: true
        )
        picker.delegate = self
        picker.allowsMultipleSelection = false
        root.present(picker, animated: true)
        pickerLog("files picker presented")
    }

    private func presentPhotos(from root: UIViewController) {
        var configuration = PHPickerConfiguration()
        configuration.filter = .any(of: [.images, .videos])
        configuration.selectionLimit = 1
        configuration.preferredAssetRepresentationMode = .current
        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = self
        root.present(picker, animated: true)
        pickerLog("photos picker presented")
    }

    // MARK: - Files

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
            pickerLog("files picked \(destination.path)")
            destination.path.withCString { rust_picker_picked($0) }
        } catch {
            pickerLog("files copy failed \(error)")
            rust_picker_cancelled()
        }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        pickerLog("files cancelled")
        rust_picker_cancelled()
    }

    // MARK: - Photos & videos

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let provider = results.first?.itemProvider else {
            pickerLog("photos cancelled")
            rust_picker_cancelled()
            return
        }

        let typeIdentifier: String
        if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) {
            typeIdentifier = UTType.movie.identifier
        } else if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
            typeIdentifier = UTType.image.identifier
        } else if let first = provider.registeredTypeIdentifiers.first {
            typeIdentifier = first
        } else {
            pickerLog("photos: no usable type")
            rust_picker_cancelled()
            return
        }

        provider.loadFileRepresentation(forTypeIdentifier: typeIdentifier) { url, error in
            guard let url else {
                pickerLog("photos load failed \(String(describing: error))")
                rust_picker_cancelled()
                return
            }
            let ext = url.pathExtension.isEmpty ? "bin" : url.pathExtension
            let base = provider.suggestedName ?? "media"
            let name = base.contains(".") ? base : "\(base).\(ext)"
            let destination = FileManager.default.temporaryDirectory
                .appendingPathComponent("\(UUID().uuidString)-\(name)")
            do {
                try FileManager.default.copyItem(at: url, to: destination)
                pickerLog("photos picked \(destination.path)")
                destination.path.withCString { rust_picker_picked($0) }
            } catch {
                pickerLog("photos copy failed \(error)")
                rust_picker_cancelled()
            }
        }
    }
}

@_cdecl("droplink_ios_pick_file")
public func droplink_ios_pick_file() {
    DropLinkPicker.shared.present()
}

/// Quick Look preview, not a share sheet: tapping a received file must show it.
final class DropLinkPreview: NSObject, QLPreviewControllerDataSource {
    static let shared = DropLinkPreview()
    private var current: QLPreviewController?
    private var url: URL?

    func present(url: URL, from root: UIViewController) {
        let controller = QLPreviewController()
        controller.dataSource = self
        self.url = url
        self.current = controller
        root.present(controller, animated: true)
    }

    func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
        url == nil ? 0 : 1
    }

    func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
        (url ?? URL(fileURLWithPath: "/")) as NSURL
    }
}

@_cdecl("droplink_ios_open_file")
public func droplink_ios_open_file(_ pathPtr: UnsafePointer<CChar>?) {
    guard let pathPtr else { return }
    let path = String(cString: pathPtr)
    pickerLog("open requested \(path)")
    DispatchQueue.main.async {
        guard let root = DropLinkPicker.rootViewController() else { return }
        DropLinkPreview.shared.present(url: URL(fileURLWithPath: path), from: root)
    }
}
