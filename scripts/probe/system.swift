import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

let deepest = 40

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
    (attribute(element, kAXChildrenAttribute as String) as? [AXUIElement] ?? []).filter {
        let role = attribute($0, kAXRoleAttribute as String) as? String
        return role != kAXApplicationRole as String && role != kAXMenuBarRole as String
    }
}

func allText(_ element: AXUIElement, depth: Int = 0) -> String {
    if depth > deepest { return "" }
    return ([text(element)] + children(element).map { allText($0, depth: depth + 1) }).joined(separator: " ")
}

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

func actionName(_ raw: String) -> String {
    guard raw.hasPrefix("Name:") else { return raw }
    return String(raw.dropFirst(5).prefix { $0 != "\n" })
}

func outline(_ element: AXUIElement, depth: Int = 0) -> [String] {
    if depth > deepest { return [] }
    let role = attribute(element, kAXRoleAttribute as String) as? String ?? "?"
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let actions = (names as? [String] ?? []).map(actionName).joined(separator: ",")
    let line = "\(String(repeating: " ", count: depth))\(role) \"\(text(element))\"" + (actions.isEmpty ? "" : " [\(actions)]")
    return [line] + children(element).flatMap { outline($0, depth: depth + 1) }
}

func actions(_ element: AXUIElement, named label: String, depth: Int = 0) -> [(AXUIElement, String)] {
    if depth > deepest { return [] }
    var names: CFArray?
    AXUIElementCopyActionNames(element, &names)
    let own = (names as? [String] ?? []).filter { $0.hasPrefix("Name:") && actionName($0) == label }.map { (element, $0) }
    return own + children(element).flatMap { actions($0, named: label, depth: depth + 1) }
}

func frame(_ element: AXUIElement) -> [String: Any] {
    var origin = CGPoint.zero
    var size = CGSize.zero
    if let place = attribute(element, kAXPositionAttribute as String) { AXValueGetValue(place as! AXValue, .cgPoint, &origin) }
    if let extent = attribute(element, kAXSizeAttribute as String) { AXValueGetValue(extent as! AXValue, .cgSize, &size) }
    return ["name": text(element), "x": origin.x, "y": origin.y, "width": size.width, "height": size.height,
            "value": (attribute(element, kAXValueAttribute as String) as? NSNumber)?.intValue ?? NSNull()]
}

let keyCodes: [String: CGKeyCode] = [
    "k": 40, "tab": 48, "return": 36, "escape": 53, "left": 123, "right": 124, "down": 125, "up": 126,
]
let keyFlags: [String: CGEventFlags] = ["cmd": .maskCommand, "shift": .maskShift]

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
case "group-frame":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 2, let pid = Int32(rest[0]) else { refuse("group-frame needs a pid and a scope") }
    let groups = scoped(AXUIElementCreateApplication(pid), rest[1])
    guard groups.count == 1 else { refuse("\(groups.count) groups on screen name \(rest[1])") }
    let screen = CGDisplayBounds(CGMainDisplayID())
    emit(["frame": frame(groups[0]),
          "screen": ["x": screen.minX, "y": screen.minY, "width": screen.width, "height": screen.height]])
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
case "find":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 3, let pid = Int32(rest[0]) else { refuse("find needs a pid, a role and a name") }
    emit(find(AXUIElementCreateApplication(pid), role: rest[1], name: rest[2]).map(frame))
case "press-one":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 3, let pid = Int32(rest[0]) else { refuse("press-one needs a pid, a role and a name") }
    let matches = find(AXUIElementCreateApplication(pid), role: rest[1], name: rest[2])
    guard matches.count == 1 else { refuse("\(matches.count) \(rest[1]) named \(rest[2]) on screen") }
    let result = AXUIElementPerformAction(matches[0], kAXPressAction as CFString)
    if result != .success { refuse("the press was refused (AXError \(result.rawValue))") }
    emit(["pressed": text(matches[0])])
case "key":
    trusted()
    let chord = args.dropFirst().first ?? ""
    let parts = chord.split(separator: "+").map(String.init)
    guard let last = parts.last, let code = keyCodes[last], parts.dropLast().allSatisfy({ keyFlags[$0] != nil })
    else { refuse("no such key: \(chord)") }
    let flags = parts.dropLast().reduce(CGEventFlags()) { $0.union(keyFlags[$1]!) }
    let source = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)
        event?.flags = flags
        event?.post(tap: .cghidEventTap)
        for _ in 0..<100 {
            if CGEventSource.keyState(.hidSystemState, key: code) == down { break }
            usleep(2_000)
        }
    }
    emit(["key": chord])
case "size":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 3, let pid = Int32(rest[0]), let width = Double(rest[1]), let height = Double(rest[2])
    else { refuse("size needs a pid, a width and a height") }
    let windows = attribute(AXUIElementCreateApplication(pid), kAXWindowsAttribute as String) as? [AXUIElement] ?? []
    guard let window = windows.max(by: { (frame($0)["height"] as! CGFloat) < (frame($1)["height"] as! CGFloat) })
    else { refuse("no window to size") }
    var size = CGSize(width: width, height: height)
    let result = AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!)
    if result != .success { refuse("the window refused the size (AXError \(result.rawValue))") }
    emit(["size": [width, height]])
case "status-item":
    trusted()
    let rest = Array(args.dropFirst())
    guard rest.count == 2, let pid = Int32(rest[0]) else { refuse("status-item needs a pid and a file") }
    if !CGPreflightScreenCaptureAccess() {
        refuse("Screen Recording is not allowed for the app that runs this command. Turn it on in System Settings, Privacy & Security, Screen & System Audio Recording.")
    }
    let bar = attribute(AXUIElementCreateApplication(pid), "AXExtrasMenuBar")
    guard let item = bar.map({ children($0 as! AXUIElement) })?.first else { refuse("no menu bar icon for \(pid)") }
    let placed = frame(item)
    let x = Int((placed["x"] as! CGFloat).rounded()), y = Int((placed["y"] as! CGFloat).rounded())
    let width = Int((placed["width"] as! CGFloat).rounded()), height = Int((placed["height"] as! CGFloat).rounded())
    guard width > 0 && height > 0 else { refuse("the menu bar icon has no size") }
    try? FileManager.default.removeItem(atPath: rest[1])
    let shot = Process()
    shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    shot.arguments = ["-x", "-R", "\(x),\(y),\(width),\(height)", rest[1]]
    try? shot.run()
    shot.waitUntilExit()
    guard shot.terminationStatus == 0, let data = try? Data(contentsOf: URL(fileURLWithPath: rest[1])),
          let image = NSBitmapImageRep(data: data)
    else { refuse("could not capture the menu bar icon") }
    var kind: [[Character]] = Array(repeating: Array(repeating: " ", count: image.pixelsWide), count: image.pixelsHigh)
    for row in 0..<image.pixelsHigh {
        for column in 0..<image.pixelsWide {
            guard let color = image.colorAt(x: column, y: row)?.usingColorSpace(.sRGB) else { continue }
            let (red, green, blue) = (color.redComponent * 255, color.greenComponent * 255, color.blueComponent * 255)
            if red > 200 && green > 140 && green < 215 && blue < 130 { kind[row][column] = "A" }
            if red < 90 && green < 90 && blue < 90 { kind[row][column] = "#" }
        }
    }
    let reach = max(3, image.pixelsWide / width * 3)
    func edged(_ line: [Character]) -> (sides: Int, outlined: Int, eye: Int) {
        guard let first = line.firstIndex(of: "A"), let last = line.lastIndex(of: "A") else { return (0, 0, 0) }
        let before = line[max(0, first - reach)..<first].contains("#") ? 1 : 0
        let after = line[(last + 1)..<min(line.count, last + 1 + reach)].contains("#") ? 1 : 0
        return (2, before + after, line[first...last].filter { $0 == "#" }.count)
    }
    let across = kind.map(edged)
    let down = (0..<image.pixelsWide).map { column in edged(kind.map { $0[column] }) }
    emit(["help": attribute(item, "AXHelp") as? String ?? "", "title": attribute(item, kAXTitleAttribute as String) as? String ?? "",
          "amber": kind.joined().filter { $0 == "A" }.count,
          "sides": (across + down).map(\.sides).reduce(0, +), "outlined": (across + down).map(\.outlined).reduce(0, +),
          "eye": across.map(\.eye).reduce(0, +)])
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
    refuse("usage: system access|front|watch-front|idle|windows|owner-windows|press|press-group|press-one|find|group-frame|describe|capture|click-icon|escape|key|size|status-item")
}
