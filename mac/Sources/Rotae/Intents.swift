import AppIntents
import AppKit
import SDCore

/// Rotae in Shortcuts and Spotlight. Only work that is safe to run without a person
/// watching is offered: the same allow-list as watched folders, checked again by the engine.
enum IntentFailure: Error, CustomLocalizedStringResourceConvertible {
    case noFiles, notAllowed, failed(String)
    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .noFiles: return "There were no files Rotae could use."
        case .notAllowed: return "This recipe has a step that needs you, so it only runs from Rotae itself."
        case .failed(let d): return "Rotae couldn't finish: \(d)"
        }
    }
}

@MainActor
enum IntentRunner {
    static func run(_ files: [IntentFile], _ action: Action, title: String) async throws -> [IntentFile] {
        guard unattendedAllowed(action) else { throw IntentFailure.notAllowed }
        var paths: [String] = [], scoped: [URL] = []
        defer { scoped.forEach { $0.stopAccessingSecurityScopedResource() } }
        for f in files {
            if let u = f.fileURL {
                if u.startAccessingSecurityScopedResource() { scoped.append(u) }
                paths.append(u.path)
            } else {
                // handed over as bytes: give it a place of its own so results have somewhere to go
                let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rotae-intent-\(UUID().uuidString)", isDirectory: true)
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let name = (f.filename as NSString).lastPathComponent
                let u = dir.appendingPathComponent(name.isEmpty || name == "." || name == ".." ? "file" : name)
                try f.data.write(to: u)
                paths.append(u.path)
            }
        }
        guard let job = Jobs.shared.queue(paths, action, title: title, unattended: true) else { throw IntentFailure.noFiles }
        do {
            while job.isActive { try await Task.sleep(nanoseconds: 50_000_000) }
        } catch {
            Jobs.shared.cancel(job) // the shortcut was stopped: stop the work too
            throw error
        }
        guard job.state == .done || job.state == .partial, !job.outputs.isEmpty || job.state == .done else {
            throw IntentFailure.failed(job.errors.first.map { explain($0.code).why } ?? "unknown error")
        }
        return job.outputs.map { IntentFile(fileURL: URL(fileURLWithPath: $0)) }
    }
}

enum SDFormat: String, AppEnum {
    case jpg, png, webp, avif, tiff, pdf, gif, mp4, webm, mov, mp3, m4a, wav, flac
    static var typeDisplayRepresentation: TypeDisplayRepresentation { "Format" }
    static var caseDisplayRepresentations: [SDFormat: DisplayRepresentation] {
        [.jpg: "JPG", .png: "PNG", .webp: "WebP", .avif: "AVIF", .tiff: "TIFF", .pdf: "PDF", .gif: "GIF", .mp4: "MP4", .webm: "WebM", .mov: "MOV", .mp3: "MP3", .m4a: "M4A", .wav: "WAV", .flac: "FLAC"]
    }
}

enum SDStrength: String, AppEnum {
    case low, medium, high
    static var typeDisplayRepresentation: TypeDisplayRepresentation { "Strength" }
    static var caseDisplayRepresentations: [SDStrength: DisplayRepresentation] { [.low: "Light", .medium: "Medium", .high: "Strong"] }
}

struct ConvertFilesIntent: AppIntent {
    static let title: LocalizedStringResource = "Convert Files"
    static let description = IntentDescription("Converts images, video, audio and documents to another format. Results are saved next to the originals, which are never changed.")
    @Parameter(title: "Files") var files: [IntentFile]
    @Parameter(title: "Format") var format: SDFormat
    static var parameterSummary: some ParameterSummary { Summary("Convert \(\.$files) to \(\.$format)") }
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        .result(value: try await IntentRunner.run(files, act("convert", ["to": .s(format.rawValue)]), title: "Converting to \(format.rawValue.uppercased())"))
    }
}

struct CompressFilesIntent: AppIntent {
    static let title: LocalizedStringResource = "Compress Files"
    static let description = IntentDescription("Makes images, video, audio and PDFs smaller.")
    @Parameter(title: "Files") var files: [IntentFile]
    @Parameter(title: "Strength", default: .medium) var strength: SDStrength
    static var parameterSummary: some ParameterSummary { Summary("Compress \(\.$files) (\(\.$strength))") }
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        .result(value: try await IntentRunner.run(files, act("compress", ["level": .s(strength.rawValue)]), title: "Compressing"))
    }
}

struct ResizeImagesIntent: AppIntent {
    static let title: LocalizedStringResource = "Resize Images"
    static let description = IntentDescription("Scales images and video to a percentage of their size.")
    @Parameter(title: "Files") var files: [IntentFile]
    @Parameter(title: "Percent", default: 50, inclusiveRange: (1, 400)) var percent: Int
    static var parameterSummary: some ParameterSummary { Summary("Resize \(\.$files) to \(\.$percent)%") }
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        .result(value: try await IntentRunner.run(files, act("resize", ["value": .n(Double(percent))]), title: "Resizing to \(percent)%"))
    }
}

struct RemoveBackgroundIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove Background"
    static let description = IntentDescription("Cuts the subject out of a photo, on this Mac.")
    @Parameter(title: "Images") var files: [IntentFile]
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        .result(value: try await IntentRunner.run(files, act("remove_bg"), title: "Removing background"))
    }
}

struct CleanMetadataIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove Metadata"
    static let description = IntentDescription("Strips location, camera and author details from photos, video and documents before you share them.")
    @Parameter(title: "Files") var files: [IntentFile]
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        .result(value: try await IntentRunner.run(files, act("strip_metadata"), title: "Removing metadata"))
    }
}

struct RecipeEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation { "Recipe" }
    static var defaultQuery: RecipeQuery { RecipeQuery() }
    let id: String
    let name: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct RecipeQuery: EntityQuery {
    /// Only recipes that can run without anyone answering a question.
    @MainActor static var all: [Recipe] {
        Store.shared.state.recipes.filter { r in r.steps.allSatisfy { unattendedAllowed($0.action) && $0.ask.isEmpty } }
    }
    @MainActor func entities(for ids: [String]) async throws -> [RecipeEntity] {
        Self.all.filter { ids.contains($0.id.uuidString) }.map { RecipeEntity(id: $0.id.uuidString, name: $0.name) }
    }
    @MainActor func suggestedEntities() async throws -> [RecipeEntity] { Self.all.map { RecipeEntity(id: $0.id.uuidString, name: $0.name) } }
}

struct RunRecipeIntent: AppIntent {
    static let title: LocalizedStringResource = "Run Recipe"
    static let description = IntentDescription("Runs one of your Rotae recipes on files.")
    @Parameter(title: "Recipe") var recipe: RecipeEntity
    @Parameter(title: "Files") var files: [IntentFile]
    static var parameterSummary: some ParameterSummary { Summary("Run \(\.$recipe) on \(\.$files)") }
    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        guard let r = RecipeQuery.all.first(where: { $0.id.uuidString == recipe.id }) else { throw IntentFailure.notAllowed }
        return .result(value: try await IntentRunner.run(files, act("chain", ["steps": .a(r.steps.map { .o($0.action) })]), title: r.name))
    }
}

struct RotaeShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: ConvertFilesIntent(), phrases: ["Convert files with \(.applicationName)"], shortTitle: "Convert Files", systemImageName: "arrow.left.arrow.right")
        AppShortcut(intent: CompressFilesIntent(), phrases: ["Compress files with \(.applicationName)"], shortTitle: "Compress Files", systemImageName: "arrow.down.right.and.arrow.up.left")
        AppShortcut(intent: RemoveBackgroundIntent(), phrases: ["Remove background with \(.applicationName)"], shortTitle: "Remove Background", systemImageName: "person.crop.rectangle")
        AppShortcut(intent: CleanMetadataIntent(), phrases: ["Remove metadata with \(.applicationName)"], shortTitle: "Remove Metadata", systemImageName: "eraser")
        AppShortcut(intent: RunRecipeIntent(), phrases: ["Run a \(.applicationName) recipe"], shortTitle: "Run Recipe", systemImageName: "bolt")
    }
}
