import Foundation
import Network
import Photos
import UIKit
import UniformTypeIdentifiers

// Rust callbacks (exported by libdroplink_ios.a).
@_silgen_name("droplink_ios_destination_changed")
func rust_destination_changed(_ name: UnsafePointer<CChar>)

@_silgen_name("droplink_ios_log")
func rust_export_log(_ message: UnsafePointer<CChar>)

func exportLog(_ message: String) {
    message.withCString { rust_export_log($0) }
}

/// Destination of received files.
///
/// - Images and videos are also saved to the Photos library.
/// - Everything else is copied to the folder the user picked in Files, or stays in the app's
///   Documents folder (visible under "On My iPhone > DropLink").
final class Destination: NSObject, UIDocumentPickerDelegate {
    static let shared = Destination()

    private let bookmarkKey = "droplink.destination.bookmark"
    private var picker: UIDocumentPickerViewController?

    func storedFolder() -> URL? {
        guard let data = UserDefaults.standard.data(forKey: bookmarkKey) else {
            return nil
        }
        var stale = false
        return try? URL(
            resolvingBookmarkData: data,
            options: [],
            relativeTo: nil,
            bookmarkDataIsStale: &stale
        )
    }

    func currentName() -> String {
        storedFolder()?.lastPathComponent ?? "DropLink (Fichiers)"
    }

    func notifyName() {
        currentName().withCString { rust_destination_changed($0) }
    }

    func pickFolder() {
        DispatchQueue.main.async {
            guard let root = DropLinkPicker.rootViewController() else { return }
            let picker = UIDocumentPickerViewController(
                forOpeningContentTypes: [UTType.folder],
                asCopy: false
            )
            picker.delegate = self
            self.picker = picker
            root.present(picker, animated: true)
            exportLog("folder picker presented")
        }
    }

    func documentPicker(
        _ controller: UIDocumentPickerViewController,
        didPickDocumentsAt urls: [URL]
    ) {
        guard let url = urls.first else { return }
        let access = url.startAccessingSecurityScopedResource()
        defer {
            if access {
                url.stopAccessingSecurityScopedResource()
            }
        }
        if let data = try? url.bookmarkData(
            options: [],
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        ) {
            UserDefaults.standard.set(data, forKey: bookmarkKey)
            exportLog("destination set to \(url.lastPathComponent)")
        }
        notifyName()
    }

    func export(_ url: URL) {
        let ext = url.pathExtension.lowercased()
        let mediaExtensions: Set<String> = [
            "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "tif", "tiff",
            "mov", "mp4", "m4v", "avi", "mkv",
        ]
        if mediaExtensions.contains(ext) {
            saveToPhotos(url)
        } else {
            copyToDestination(url)
        }
    }

    private func saveToPhotos(_ url: URL) {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
            guard status == .authorized || status == .limited else {
                exportLog("photos permission denied")
                return
            }
            PHPhotoLibrary.shared().performChanges {
                if UTType(filenameExtension: url.pathExtension)?.conforms(to: .movie) == true {
                    PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: url)
                } else {
                    PHAssetChangeRequest.creationRequestForAssetFromImage(atFileURL: url)
                }
            } completionHandler: { success, error in
                exportLog("photos save success=\(success) error=\(String(describing: error))")
            }
        }
    }

    private func copyToDestination(_ url: URL) {
        guard let folder = storedFolder() else {
            exportLog("no destination folder, kept in Documents")
            return
        }
        let access = folder.startAccessingSecurityScopedResource()
        defer {
            if access {
                folder.stopAccessingSecurityScopedResource()
            }
        }
        let target = folder.appendingPathComponent(url.lastPathComponent)
        try? FileManager.default.removeItem(at: target)
        do {
            try FileManager.default.copyItem(at: url, to: target)
            exportLog("copied to \(target.path)")
        } catch {
            exportLog("copy failed: \(error)")
        }
    }
}

@_cdecl("droplink_ios_pick_folder")
public func droplink_ios_pick_folder() {
    Destination.shared.pickFolder()
}

@_cdecl("droplink_ios_open_files")
public func droplink_ios_open_files() {
    DispatchQueue.main.async {
        if let url = URL(string: "shareddocuments://") {
            UIApplication.shared.open(url)
        }
    }
}

@_cdecl("droplink_ios_export")
public func droplink_ios_export(_ pathPtr: UnsafePointer<CChar>?) {
    guard let pathPtr else { return }
    let url = URL(fileURLWithPath: String(cString: pathPtr))
    Destination.shared.export(url)
}
