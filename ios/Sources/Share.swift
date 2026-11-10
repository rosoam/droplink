import UIKit

/// Present the system share sheet for a short text (the session code).
@_cdecl("droplink_ios_share")
public func droplink_ios_share(_ textPtr: UnsafePointer<CChar>?) {
    guard let textPtr else { return }
    let text = String(cString: textPtr)
    DispatchQueue.main.async {
        guard let root = DropLinkPicker.rootViewController() else { return }
        let controller = UIActivityViewController(activityItems: [text], applicationActivities: nil)
        if let popover = controller.popoverPresentationController {
            popover.sourceView = root.view
            popover.sourceRect = CGRect(
                x: root.view.bounds.midX,
                y: root.view.bounds.midY,
                width: 0,
                height: 0
            )
        }
        root.present(controller, animated: true)
    }
}
