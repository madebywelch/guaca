// What macOS knows about the test app that the app itself cannot say, for
// scripts/probe.py. Compiled by the probe on first use; no dependencies.
//
//   access                     whether this process may use Accessibility and
//                              capture the screen, without asking for either
//   front                      the frontmost app
//   watch-front                one line per app that becomes frontmost, until
//                              stdin closes
//   idle                       seconds since the last keyboard or pointer event
//   windows <pid>              that process's windows on screen: id, layer, frame
//   owner-windows <name>       the same, for a process by name
//   press <pid> <role> <name> <scope>
//                              presses the one element with that role and name
//                              inside a group whose text contains <scope>
//   press-group <pid> <scope>  presses the group itself (a banner)
//   capture <pid> <file>       one PNG of each of that process's windows
//
// Every answer is one JSON value on stdout. A refusal is {"error": "..."} and
// exit status 2, so the caller can tell "no" from "could not ask".

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

func emit(_ value: Any) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
}

func refuse(_ why: String) -> Never {
    emit(["error": why])
    exit(2)
}

func appSummary(_ app: NSRunningApplication?) -> [String: Any] {
    guard let app else { return ["bundle": NSNull(), "pid": NSNull()] }
    return ["bundle": app.bundleIdentifier ?? NSNull(), "pid": Int(app.processIdentifier)]
}

func windows(where keep: ([String: Any]) -> Bool) -> [[String: Any]] {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    return list.filter(keep).map { info in
        let bounds = info[kCGWindowBounds as String] as? [String: Any] ?? [:]
        return [
            "id": info[kCGWindowNumber as String] as? Int ?? 0,
            "layer": info[kCGWindowLayer as String] as? Int ?? 0,
            "pid": info[kCGWindowOwnerPID as String] as? Int ?? 0,
            "x": bounds["X"] ?? 0, "y": bounds["Y"] ?? 0,
            "width": bounds["Width"] ?? 0, "height": bounds["Height"] ?? 0,
        ]
    }
}

func attribute(_ element: AXUIElement, _ name: String) -> Any? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func text(_ element: AXUIElement) -> String {
    [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, kAXIdentifierAttribute]
        .compactMap { attribute(element, $0 as String) as? String }
        .joined(separator: " ")
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute as String) as? [AXUIElement] ?? []
}

/// Every text below an element, so a group can be recognized by what it says.
func allText(_ element: AXUIElement, depth: Int = 0) -> String {
    if depth > 12 { return "" }
    return ([text(element)] + children(element).map { allText($0, depth: depth + 1) }).joined(separator: " ")
}

/// Groups whose text names the scope, innermost first: the banner or prompt
/// that belongs to the test app, and never one that belongs to another.
func scoped(_ root: AXUIElement, _ scope: String, depth: Int = 0) -> [AXUIElement] {
    if depth > 12 { return [] }
    let inner = children(root).flatMap { scoped($0, scope, depth: depth + 1) }
    if !inner.isEmpty { return inner }
    let role = attribute(root, kAXRoleAttribute as String) as? String ?? ""
    return role == "AXGroup" && allText(root).contains(scope) ? [root] : []
}

func find(_ root: AXUIElement, role: String, name: String, depth: Int = 0) -> [AXUIElement] {
    if depth > 12 { return [] }
    var found: [AXUIElement] = []
    let own = attribute(root, kAXRoleAttribute as String) as? String ?? ""
    if own == role && text(root).split(separator: " ").joined(separator: " ").contains(name) {
        found.append(root)
    }
    return found + children(root).flatMap { find($0, role: role, name: name, depth: depth + 1) }
}

func trusted() {
    if !AXIsProcessTrusted() {
        refuse("Accessibility is not allowed for the app that runs this command. Turn it on in System Settings, Privacy & Security, Accessibility.")
    }
}

let args = CommandLine.arguments.dropFirst()
switch args.first {
case "access":
    emit(["accessibility": AXIsProcessTrusted(), "screenCapture": CGPreflightScreenCaptureAccess()])
case "front":
    emit(appSummary(NSWorkspace.shared.frontmostApplication))
case "watch-front":
    let center = NSWorkspace.shared.notificationCenter
    center.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { note in
        var line = appSummary(note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)
        line["at"] = Date().timeIntervalSince1970
        emit(line)
    }
    FileHandle.standardInput.readabilityHandler = { handle in
        if handle.availableData.isEmpty { exit(0) }
    }
    emit(["watching": true, "front": appSummary(NSWorkspace.shared.frontmostApplication)])
    RunLoop.main.run()
case "idle":
    let any = CGEventType(rawValue: ~0)!
    emit(["seconds": CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: any)])
case "windows":
    guard let pid = Int(args.dropFirst().first ?? "") else { refuse("windows needs a pid") }
    emit(windows { ($0[kCGWindowOwnerPID as String] as? Int) == pid })
case "owner-windows":
    let name = args.dropFirst().first ?? ""
    emit(windows { ($0[kCGWindowOwnerName as String] as? String) == name })
case "press", "press-group":
    trusted()
    let rest = Array(args.dropFirst())
    guard let pid = Int32(rest.first ?? "") else { refuse("press needs a pid") }
    let app = AXUIElementCreateApplication(pid)
    let scope = rest.last ?? ""
    let groups = scoped(app, scope)
    if groups.isEmpty { refuse("nothing on screen names \(scope)") }
    if groups.count > 1 { refuse("\(groups.count) groups on screen name \(scope)") }
    var target = groups[0]
    if args.first == "press" {
        guard rest.count == 4 else { refuse("press needs a pid, a role, a name and a scope") }
        let matches = find(groups[0], role: rest[1], name: rest[2])
        if matches.count != 1 { refuse("\(matches.count) \(rest[1]) named \(rest[2]) inside the group naming \(scope)") }
        target = matches[0]
    }
    let result = AXUIElementPerformAction(target, kAXPressAction as CFString)
    if result != .success { refuse("the press was refused (AXError \(result.rawValue))") }
    emit(["pressed": text(target)])
case "capture":
    let rest = Array(args.dropFirst())
    guard rest.count == 2, let pid = Int(rest[0]) else { refuse("capture needs a pid and a file") }
    if !CGPreflightScreenCaptureAccess() {
        refuse("Screen Recording is not allowed for the app that runs this command. Turn it on in System Settings, Privacy & Security, Screen & System Audio Recording.")
    }
    var files: [String] = []
    for (index, window) in windows(where: { ($0[kCGWindowOwnerPID as String] as? Int) == pid && ($0[kCGWindowLayer as String] as? Int ?? 1) >= 0 }).enumerated() {
        let file = rest[1].replacingOccurrences(of: ".png", with: "-\(index).png")
        let shot = Process()
        shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        shot.arguments = ["-x", "-o", "-l\(window["id"] as! Int)", file]
        try? shot.run()
        shot.waitUntilExit()
        if shot.terminationStatus == 0 { files.append(file) }
    }
    emit(["files": files])
default:
    refuse("usage: system access|front|watch-front|idle|windows|owner-windows|press|press-group|capture")
}
