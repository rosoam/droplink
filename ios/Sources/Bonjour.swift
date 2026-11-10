import Foundation
import Network

// Rust entry points (exported by libdroplink_ios.a).
@_silgen_name("droplink_ios_peer_found")
func rust_peer_found(_ id: UnsafePointer<CChar>, _ alias: UnsafePointer<CChar>, _ ip: UnsafePointer<CChar>, _ port: UInt16)

@_silgen_name("droplink_ios_peer_lost")
func rust_peer_lost(_ id: UnsafePointer<CChar>)

@_silgen_name("droplink_ios_log")
func rust_log(_ message: UnsafePointer<CChar>)

func log(_ message: String) {
    message.withCString { rust_log($0) }
}

/// Bonjour discovery for iOS.
///
/// Publishing uses `NetService` (no socket needed, no multicast entitlement). Browsing uses
/// `NWBrowser` and reads the peer address from the TXT record, because resolving a `.local`
/// hostname requires multicast queries that iOS blocks without the multicast entitlement.
final class Bonjour: NSObject, NetServiceDelegate {
    static let shared = Bonjour()

    private var publisher: NetService?
    private var browser: NWBrowser?
    private var publishedName: String?
    private var idByService: [String: String] = [:]

    func start(port: UInt16, deviceId: String, alias: String, ip: String) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.stopLocked()

            let short = String(deviceId.prefix(6))
            let name = "\(alias)-\(short)"
            let service = NetService(
                domain: "local.",
                type: "_droplink._tcp.",
                name: name,
                port: Int32(port)
            )
            let txt: [String: Data] = [
                "id": Data(deviceId.utf8),
                "alias": Data(alias.utf8),
                "v": Data("2".utf8),
                "ip": Data(ip.utf8),
                "port": Data(String(port).utf8),
            ]
            service.setTXTRecord(NetService.data(fromTXTRecord: txt))
            service.delegate = self
            service.schedule(in: .main, forMode: .common)
            service.publish()
            self.publisher = service
            self.publishedName = service.name
            log("bonjour publish requested name=\(name) port=\(port) ip=\(ip)")

            let parameters = NWParameters.tcp
            let browser = NWBrowser(
                for: .bonjour(type: "_droplink._tcp", domain: nil),
                using: parameters
            )
            browser.stateUpdateHandler = { state in
                log("bonjour browser state \(state)")
            }
            browser.browseResultsChangedHandler = { [weak self] _, changes in
                guard let self else { return }
                for change in changes {
                    switch change {
                    case .added(let result):
                        self.handle(result)
                    case .removed(let result):
                        self.remove(result)
                    case .changed(let old, let new, _):
                        self.remove(old)
                        self.handle(new)
                    default:
                        break
                    }
                }
            }
            browser.start(queue: .main)
            self.browser = browser
        }
    }

    func stop() {
        DispatchQueue.main.async { [weak self] in
            self?.stopLocked()
        }
    }

    private func stopLocked() {
        publisher?.stop()
        publisher = nil
        browser?.cancel()
        browser = nil
        publishedName = nil
        idByService.removeAll()
    }

    private func serviceName(_ result: NWBrowser.Result) -> String? {
        if case let .service(name, _, _, _) = result.endpoint {
            return name
        }
        return nil
    }

    private func handle(_ result: NWBrowser.Result) {
        guard case let .bonjour(txt) = result.metadata else { return }

        func value(_ key: String) -> String? {
            switch txt.getEntry(for: key) {
            case .string(let string):
                return string
            case .data(let data):
                return String(data: data, encoding: .utf8)
            default:
                return nil
            }
        }

        guard
            let id = value("id"),
            value("v") == "2",
            let ip = value("ip"),
            let portValue = value("port"),
            let port = UInt16(portValue),
            port > 0
        else {
            log("bonjour result without usable TXT: \(result.endpoint)")
            return
        }
        let alias = value("alias") ?? "Appareil"
        let name = serviceName(result) ?? result.endpoint.debugDescription
        if name == publishedName {
            return
        }
        idByService[name] = id
        log("bonjour peer id=\(id) alias=\(alias) ip=\(ip) port=\(port)")
        id.withCString { idPtr in
            alias.withCString { aliasPtr in
                ip.withCString { ipPtr in
                    rust_peer_found(idPtr, aliasPtr, ipPtr, port)
                }
            }
        }
    }

    private func remove(_ result: NWBrowser.Result) {
        let name = serviceName(result) ?? result.endpoint.debugDescription
        if let id = idByService.removeValue(forKey: name) {
            id.withCString { rust_peer_lost($0) }
        }
    }

    // MARK: - NetServiceDelegate

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        log("bonjour publish failed \(errorDict)")
    }
}

@_cdecl("droplink_ios_start_bonjour")
public func droplink_ios_start_bonjour(
    _ port: UInt16,
    _ deviceIdPtr: UnsafePointer<CChar>?,
    _ aliasPtr: UnsafePointer<CChar>?,
    _ ipPtr: UnsafePointer<CChar>?
) {
    guard let deviceIdPtr, let aliasPtr, let ipPtr else { return }
    let deviceId = String(cString: deviceIdPtr)
    let alias = String(cString: aliasPtr)
    let ip = String(cString: ipPtr)
    Bonjour.shared.start(port: port, deviceId: deviceId, alias: alias, ip: ip)
}

@_cdecl("droplink_ios_stop_bonjour")
public func droplink_ios_stop_bonjour() {
    Bonjour.shared.stop()
}
