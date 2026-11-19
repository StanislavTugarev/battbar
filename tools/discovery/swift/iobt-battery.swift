// SPDX-License-Identifier: GPL-3.0-or-later
// Stage 0: what does IOBluetooth expose about paired devices and their battery?
// Read-only, prints no addresses. Uses KVC on (private) battery properties so the same
// selectors can later be called from Rust via objc2.
import Foundation
import IOBluetooth

let keys = ["batteryPercentSingle", "batteryPercentCombined", "batteryPercentLeft",
            "batteryPercentRight", "batteryPercentCase", "headsetBattery",
            "batteryLevel", "batteryPercent"]

guard let devices = IOBluetoothDevice.pairedDevices() as? [IOBluetoothDevice] else {
    print("pairedDevices() returned nil (Bluetooth permission?)"); exit(1)
}
for d in devices {
    var line = "\(d.name ?? "?") | connected=\(d.isConnected()) | class=0x\(String(d.classOfDevice, radix: 16))"
    for k in keys where d.responds(to: NSSelectorFromString(k)) {
        line += " | \(k)=\(String(describing: d.value(forKey: k) ?? "nil"))"
    }
    print(line)
}
