// SPDX-License-Identifier: GPL-3.0-or-later
// Stage 0: read the standard BLE Battery Service (0x180F / 0x2A19) of peripherals that
// are ALREADY connected to this Mac. No scanning: only retrieveConnectedPeripherals.
// Also lists the GATT services the app is allowed to see (identifiers only).
import Foundation
import CoreBluetooth

final class Probe: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var cm: CBCentralManager!
    var peripherals: [CBPeripheral] = []
    var pending = 0

    override init() { super.init(); cm = CBCentralManager(delegate: self, queue: nil) }

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        print("central state: \(c.state.rawValue) (5 = poweredOn), authorization: \(CBManager.authorization.rawValue) (3 = allowedAlways)")
        guard c.state == .poweredOn else { exit(1) }
        // Battery service, HID service, Device Information: whichever the system reports.
        let svcs = [CBUUID(string: "180F"), CBUUID(string: "1812"), CBUUID(string: "180A")]
        peripherals = c.retrieveConnectedPeripherals(withServices: svcs)
        print("connected peripherals: \(peripherals.count)")
        if peripherals.isEmpty { exit(0) }
        for p in peripherals { p.delegate = self; pending += 1; c.connect(p) }
    }
    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        print("[\(p.name ?? "?")] connected (app-level); discovering services")
        p.discoverServices(nil)
    }
    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        print("[\(p.name ?? "?")] connect failed: \(String(describing: error))"); done()
    }
    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        let s = p.services ?? []
        print("[\(p.name ?? "?")] services visible to app: \(s.map { $0.uuid.uuidString })")
        if let e = error { print("  error: \(e)") }
        if let b = s.first(where: { $0.uuid == CBUUID(string: "180F") }) {
            p.discoverCharacteristics([CBUUID(string: "2A19")], for: b)
        } else { done() }
        for svc in s where svc.uuid != CBUUID(string: "180F") { p.discoverCharacteristics(nil, for: svc) }
    }
    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        if s.uuid == CBUUID(string: "180F") {
            for ch in s.characteristics ?? [] {
                print("  battery char \(ch.uuid.uuidString) props=0x\(String(ch.properties.rawValue, radix: 16))")
                p.readValue(for: ch)
            }
        } else {
            print("  service \(s.uuid.uuidString) chars: \((s.characteristics ?? []).map { "\($0.uuid.uuidString)(0x\(String($0.properties.rawValue, radix: 16)))" })")
        }
    }
    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        if let v = ch.value, let b = v.first { print("[\(p.name ?? "?")] Battery Level = \(b)%") }
        if let e = error { print("  read error: \(e)") }
        done()
    }
    func done() {
        pending -= 1
        if pending <= 0 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { exit(0) }
        }
    }
}

let probe = Probe()
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { print("timeout"); exit(2) }
RunLoop.main.run()
