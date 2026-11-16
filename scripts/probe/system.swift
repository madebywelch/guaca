// What macOS knows about the test app that the app itself cannot say, for
// scripts/probe.py. Compiled by the probe on first use; no dependencies.
//
//   access                     whether this process may use Accessibility and
//                              capture the screen, without asking for either,
//                              and whether the screen is locked
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
//   describe <pid> <scope>     what each group naming <scope> holds: roles,
//                              text and actions, and nothing outside it
//   capture <pid> <file>       one PNG of each of that process's windows
//   click-icon <pid>           a real click, through the window server, on
//                              that process's menu bar icon; the pointer is
//                              put back where it was
//   escape                     a real press of the Escape key
//
// Every answer is one JSON value on stdout. A refusal is {"error": "..."} and
// exit status 2, so the caller can tell "no" from "could not ask".

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

/// How far below an app's root an element is looked for.
let deepest = 30

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

/// An element's children, without the menu bar and without application
/// elements. On macOS 27 Notification Center can list application elements among
/// its own children that lead back to itself; followed, every level multiplies
/// the walk, and none of it is a banner or a question.
func children(_ element: AXUIElement) -> [AXUIElement] {
    (attribute(element, kAXChildrenAttribute as String) as? [AXUIElement] ?? []).filter {
        let role = attribute($0, kAXRoleAttribute as String) as? String
        return role != kAXApplicationRole as String && role != kAXMenuBarRole as String
    }
}

/// Every text below an element, so a group can be recognized by what it says.
func allText(_ element: AXUIElement, depth: Int = 0) -> String {
    if depth > deepest { return "" }
    return ([text(element)] + children(element).map { allText($0, depth: depth + 1) }).joined(separator: " ")
}

/// Groups whose text names the scope, innermost first: the banner or prompt
/// that belongs to the test app, and never one that belongs to another.
func scoped(_ root: AXUIElement, _ scope: String, depth: Int = 0) -> [AXUIElement] {
    if depth > deepest { return [] }
    let inner = children(root).flatMap { scoped($0, scope, depth: depth + 1) }
    if !inner.isEmpty { return inner }
    let role = attribute(root, kAXRoleAttribute as String) as? String ?? ""
    return role == "AXGroup" && allText(root).contains(scope) ? [root] : []
}

func find(_ root: AXUIElement, role: String, name: String, depth: Int = 0) -> [AXUIElement] {
    if depth > deepest { return [] }
    var found: [AXUIElement] = []
    let own = attribute(root, kAXRoleAttribute as String) as? String ?? ""
    if own == role && text(root).split(separator: " ").joined(separator: " ").contains(name) {
        found.append(root)
    }
    return found + children(root).flatMap { find($0, role: role, name: name, depth: depth + 1) }
}

/// An action as it reads: Notification Center names its own actions
/// "Name:Allow\nTarget:0x0\nSelector:(null)", and the name is the label.
func actionName(_ raw: String) -> String {
    guard raw.hasPrefix("Name:") else { return raw }
    return String(raw.dropFirst(5).prefix { $0 != "\n" })
}

/// What is inside a group, as role, text and actions, so a press that found
/// nothing says what it did find.
func outline(_ element: AXUIElement, depth: Int = 0) -> [String] {
    if depth > deepest { return [] }
    let role = attribute(element, kAXRoleAttribute as String) as? String ?? "?"
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let actions = (names as? [String] ?? []).map(actionName).joined(separator: ",")
    let line = "\(String(repeating: " ", count: depth))\(role) \"\(text(element))\"" + (actions.isEmpty ? "" : " [\(actions)]")
    return [line] + children(element).flatMap { outline($0, depth: depth + 1) }
}

/// Elements below a group that offer an action with that label, and the
/// action's raw name.
func actions(_ element: AXUIElement, named label: String, depth: Int = 0) -> [(AXUIElement, String)] {
    if depth > deepest { return [] }
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let own = (names as? [String] ?? []).filter { $0.hasPrefix("Name:") && actionName($0) == label }.map { (element, $0) }
    return own + children(element).flatMap { actions($0, named: label, depth: depth + 1) }
}

func trusted() {
    if !AXIsProcessTrusted() {
        refuse("Accessibility is not allowed for the app that runs this command. Turn it on in System Settings, Privacy & Security, Accessibility.")
    }
}

let args = CommandLine.arguments.dropFirst()
switch args.first {
case "access":
    let session = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
    emit([
        "accessibility": AXIsProcessTrusted(),
        "screenCapture": CGPreflightScreenCaptureAccess(),
        "locked": session["CGSSessionScreenIsLocked"] as? Bool ?? false,
    ])
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
    // Keys, the pointer and the wheel, and nothing else: a display waking,
    // which the probe does itself, resets the count of any event with
    // nobody at the Mac.
    let input: [CGEventType] = [
        .keyDown, .flagsChanged, .mouseMoved, .scrollWheel,
        .leftMouseDown, .rightMouseDown, .otherMouseDown,
        .leftMouseDragged, .rightMouseDragged, .otherMouseDragged,
    ]
    emit(["seconds": input.map { CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: $0) }.min()!])
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
    var action = kAXPressAction as String
    if args.first == "press" {
        guard rest.count == 4 else { refuse("press needs a pid, a role, a name and a scope") }
        let matches = find(groups[0], role: rest[1], name: rest[2])
        // macOS 27 draws the question's buttons only under the pointer; to
        // assistive technology they are named actions on the group instead.
        let named = actions(groups[0], named: rest[2])
        if matches.count == 1 {
            target = matches[0]
        } else if matches.isEmpty && named.count == 1 {
            (target, action) = named[0]
        } else {
            refuse("\(matches.count) \(rest[1]) and \(named.count) actions named \(rest[2]) inside the group naming \(scope); it holds: "
                + outline(groups[0]).joined(separator: " | "))
        }
    }
    let result = AXUIElementPerformAction(target, action as CFString)
    if result != .success { refuse("the press was refused (AXError \(result.rawValue))") }
    emit(["pressed": text(target), "action": actionName(action)])
case "describe":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 2, let pid = Int32(rest[0]) else { refuse("describe needs a pid and a scope") }
    emit(scoped(AXUIElementCreateApplication(pid), rest[1]).map { outline($0).joined(separator: " | ") })
case "click-icon":
    trusted()
    guard let pid = Int32(args.dropFirst().first ?? "") else { refuse("click-icon needs a pid") }
    let bar = attribute(AXUIElementCreateApplication(pid), "AXExtrasMenuBar")
    guard let item = bar.map({ children($0 as! AXUIElement) })?.first,
          let place = attribute(item, kAXPositionAttribute as String),
          let extent = attribute(item, kAXSizeAttribute as String)
    else { refuse("no menu bar icon for \(pid)") }
    var origin = CGPoint.zero
    var size = CGSize.zero
    AXValueGetValue(place as! AXValue, .cgPoint, &origin)
    AXValueGetValue(extent as! AXValue, .cgSize, &size)
    let at = CGPoint(x: origin.x + size.width / 2, y: origin.y + size.height / 2)
    let back = CGEvent(source: nil)?.location ?? at
    let source = CGEventSource(stateID: .hidSystemState)
    for (type, pause) in [(CGEventType.mouseMoved, 80_000), (.leftMouseDown, 60_000), (.leftMouseUp, 80_000)] {
        CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: at, mouseButton: .left)?
            .post(tap: .cghidEventTap)
        usleep(useconds_t(pause))
    }
    CGWarpMouseCursorPosition(back)
    emit(["clicked": [at.x, at.y]])
case "escape":
    trusted()
    let source = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        CGEvent(keyboardEventSource: source, virtualKey: 53, keyDown: down)?.post(tap: .cghidEventTap)
        usleep(40_000)
    }
    emit(["escape": true])
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
    refuse("usage: system access|front|watch-front|idle|windows|owner-windows|press|press-group|describe|capture|click-icon|escape")
}
