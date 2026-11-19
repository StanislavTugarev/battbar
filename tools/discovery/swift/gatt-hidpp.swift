// SPDX-License-Identifier: GPL-3.0-or-later
// Stage 0: is HID++ 2.0 reachable over the Logitech vendor GATT service of a connected
// BLE peripheral (no HID device opened => no keystrokes can be received)?
// Read-only requests only: ping, IRoot.getFeature, FeatureSet, DeviceInformation, battery getters.
import Foundation
import CoreBluetooth

let SVC = CBUUID(string: "00010000-0000-1000-8000-011F2000046D")
let CHR = CBUUID(string: "00010001-0000-1000-8000-011F2000046D")
let SW: UInt8 = 0x0B

func hex(_ d: [UInt8]) -> String { d.map { String(format: "%02x", $0) }.joined(separator: " ") }
func fp(_ b: [UInt8]) -> String {
    var h: UInt64 = 0xcbf29ce484222325
    for x in b { h ^= UInt64(x); h = h &* 0x100000001b3 }
    return String(format: "<fp:%06x>", h & 0xFFFFFF)
}

final class P: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var cm: CBCentralManager!
    var per: CBPeripheral?
    var ch: CBCharacteristic?
    var withReportId = false
    var variant = 0
    // (reportId?, writeType, deviceIndex (0x100 = omitted), total length)
    let variants: [(Bool, CBCharacteristicWriteType, Int, Int)] = [
        (false, .withResponse, 0x100, 18), (false, .withResponse, 0xFF, 18), (false, .withResponse, 0x100, 17),
        (false, .withResponse, 0x100, 16), (false, .withResponse, 0x100, 5), (false, .withResponse, 0xFF, 7),
        (false, .withResponse, 0x100, 19), (false, .withResponse, 0x100, 20)]
    var total = 19
    var wtype: CBCharacteristicWriteType = .withResponse
    var idx: Int = 0xFF
    var waiting: ((([UInt8])?) -> Void)?
    var timer: DispatchWorkItem?

    override init() { super.init(); cm = CBCentralManager(delegate: self, queue: nil) }
    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        guard c.state == .poweredOn else { return }
        guard let p = c.retrieveConnectedPeripherals(withServices: [SVC]).first else { print("no connected peripheral with Logitech GATT service"); exit(0) }
        per = p; p.delegate = self; print("peripheral: \(p.name ?? "?")"); c.connect(p)
    }
    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) { p.discoverServices([SVC]) }
    func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
        guard let s = p.services?.first(where: { $0.uuid == SVC }) else { print("service missing"); exit(1) }
        p.discoverCharacteristics([CHR], for: s)
    }
    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        guard let c = s.characteristics?.first(where: { $0.uuid == CHR }) else { print("char missing"); exit(1) }
        ch = c; p.setNotifyValue(true, for: c)
    }
    func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor c: CBCharacteristic, error: Error?) {
        print("notify enabled: \(c.isNotifying) \(error.map { "\($0)" } ?? "")")
        run()
    }
    func peripheral(_ p: CBPeripheral, didUpdateValueFor c: CBCharacteristic, error: Error?) {
        let v = [UInt8](c.value ?? Data())
        if let w = waiting { timer?.cancel(); waiting = nil; w(v) } else { print("  unsolicited: \(hex(v))") }
    }

    // Send one HID++ long message (device index 0xFF). Layout candidates:
    //  A) 19 bytes: [idx, feat, fn|sw, params...]   B) 20 bytes: [0x11, idx, feat, fn|sw, params...]
    func req(_ feat: UInt8, _ fn: UInt8, _ params: [UInt8], _ cb: @escaping ([UInt8]?) -> Void) {
        var m: [UInt8] = [feat, (fn << 4) | SW] + params
        if idx <= 0xFF { m.insert(UInt8(idx), at: 0) }
        if withReportId { m.insert(0x11, at: 0) }
        m += [UInt8](repeating: 0, count: max(0, total - m.count))
        if m.count > total { m = Array(m.prefix(total)) }
        waiting = { r in
            guard var r = r else { cb(nil); return }
            if self.withReportId, r.first == 0x11 { r.removeFirst() }
            // r = [idx, feat, fn|sw, data...]  or error [idx, 0xFF, feat, fn|sw, err]
            print("  <- \(hex(r))")
            let off = self.idx <= 0xFF ? 1 : 0
            if r.count >= off + 4, r[off] == 0xFF, r[off + 1] == feat { print("  HID++ error 0x\(String(format: "%02x", r[off + 3])) for feat 0x\(String(format: "%02x", feat))"); cb(nil); return }
            cb(Array(r.dropFirst(off + 2)))
        }
        let t = DispatchWorkItem { [weak self] in if let w = self?.waiting { self?.waiting = nil; w(nil) } }
        timer = t; DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: t)
        print("  -> \(hex(m))"); per!.writeValue(Data(m), for: ch!, type: wtype)
    }

    func applyVariant() {
        let v = variants[variant]; withReportId = v.0; wtype = v.1; idx = v.2; total = v.3
        print("variant \(variant): reportId=\(v.0) idx=\(v.2 > 0xFF ? "omitted" : String(format: "0x%02x", v.2)) length=\(v.3)")
    }
    func peripheral(_ p: CBPeripheral, didWriteValueFor c: CBCharacteristic, error: Error?) {
        if let e = error { print("  write error: \(e)") }
    }
    func run() {
        if variant == 0 && waiting == nil && timer == nil { applyVariant() }
        req(0x00, 1, [0, 0, 0xAA]) { r in
            guard let r = r else {
                self.variant += 1
                if self.variant < self.variants.count { self.applyVariant(); self.run(); return }
                print("no HID++ reply over GATT in any variant"); exit(0)
            }
            print("framing: variant \(self.variant), length \(self.total), idx \(self.idx > 0xFF ? "omitted" : "present")")
            print("ping -> HID++ \(r[0]).\(r[1]) echo=0x\(String(format: "%02x", r[2]))")
            self.features()
        }
    }
    func getFeature(_ id: UInt16, _ cb: @escaping (UInt8?) -> Void) {
        req(0x00, 0, [UInt8(id >> 8), UInt8(id & 0xFF)]) { r in cb(r.flatMap { $0[0] == 0 ? nil : $0[0] }) }
    }
    func features() {
        getFeature(0x0001) { fs in
            guard let fs = fs else { print("no FeatureSet"); exit(0) }
            self.req(fs, 0, []) { r in
                let n = Int(r?[0] ?? 0); print("features (\(n) + root):")
                var list: [String] = []
                func next(_ i: Int) {
                    if i > n { print("  " + list.joined(separator: ", ")); self.info(); return }
                    self.req(fs, 1, [UInt8(i)]) { r in
                        if let r = r { list.append(String(format: "0x%02x:%04x%@", i, (UInt16(r[0]) << 8) | UInt16(r[1]), r[2] & 0x40 != 0 ? "(h)" : "")) }
                        next(i + 1)
                    }
                }
                next(0)
            }
        }
    }
    func info() {
        getFeature(0x0003) { fi in
            guard let fi = fi else { self.battery(); return }
            self.req(fi, 0, []) { r in
                if let r = r {
                    print("DeviceInformation: unitId=\(fp(Array(r[1..<5]))) transports=0x\(String(format: "%02x%02x", r[5], r[6])) modelIds=\(hex(Array(r[7..<13])))")
                }
                self.battery()
            }
        }
    }
    func battery() {
        getFeature(0x1004) { u in
            if let u = u { self.req(u, 1, []) { r in print("0x1004 status: \(hex(r ?? []))"); exit(0) }; return }
            self.getFeature(0x1001) { v in
                guard let v = v else { print("no battery feature"); exit(0) }
                self.req(v, 0, []) { r in
                    if let r = r { print("0x1001 BatteryVoltage: \((UInt16(r[0]) << 8) | UInt16(r[1])) mV flags=0x\(String(format: "%02x", r[2]))") }
                    exit(0)
                }
            }
        }
    }
}
let p = P()
DispatchQueue.main.asyncAfter(deadline: .now() + 40) { print("global timeout"); exit(2) }
RunLoop.main.run()
