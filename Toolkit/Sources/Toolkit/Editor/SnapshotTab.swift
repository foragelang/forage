import SwiftUI
import Forage

/// Renders `RunResultStore.latest?.snapshot`. Top section: record counts by
/// `typeName`. Selecting a type expands to a table of records' fields.
struct SnapshotTab: View {
    let slug: String

    @Environment(RunResultStore.self) private var runResults
    @State private var selectedType: String?

    var body: some View {
        Group {
            if let snapshot = runResults.snapshot(for: slug) {
                content(snapshot)
            } else {
                ContentUnavailableView(
                    "No snapshot yet",
                    systemImage: "doc.text.below.ecg",
                    description: Text("Use \"Run live\" or \"Run replay\" to produce a snapshot.")
                )
            }
        }
    }

    @ViewBuilder
    private func content(_ snapshot: Snapshot) -> some View {
        HSplitView {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("\(snapshot.records.count) records")
                        .font(.headline)
                    Spacer()
                    Text(snapshot.observedAt.formatted(date: .abbreviated, time: .shortened))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(10)
                Divider()
                typeList(snapshot)
            }
            .frame(minWidth: 220, idealWidth: 260, maxWidth: 320)

            recordTable(snapshot)
                .frame(minWidth: 460)
        }
    }

    private func typeList(_ snapshot: Snapshot) -> some View {
        let counts = countsByType(snapshot.records)
        return List(counts, id: \.typeName, selection: $selectedType) { tc in
            HStack {
                Text(tc.typeName)
                    .font(.system(size: 12, design: .monospaced))
                Spacer()
                Text("\(tc.count)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            .tag(tc.typeName)
        }
        .listStyle(.sidebar)
    }

    @ViewBuilder
    private func recordTable(_ snapshot: Snapshot) -> some View {
        if let typeName = selectedType ?? snapshot.records.first?.typeName {
            let records = snapshot.records.filter { $0.typeName == typeName }
            ScrollView([.horizontal, .vertical]) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(typeName)
                        .font(.headline)
                        .padding(.horizontal, 10)
                        .padding(.top, 10)
                    if let first = records.first {
                        let columns = Array(first.fields.keys).sorted()
                        recordsGrid(records: records, columns: columns)
                            .padding(.horizontal, 10)
                            .padding(.bottom, 10)
                    } else {
                        Text("No records.").foregroundStyle(.secondary).padding()
                    }
                }
            }
        } else {
            ContentUnavailableView(
                "Pick a type",
                systemImage: "tablecells",
                description: Text("Select a record type from the list to inspect rows.")
            )
        }
    }

    private func recordsGrid(records: [ScrapedRecord], columns: [String]) -> some View {
        Grid(alignment: .topLeading, horizontalSpacing: 14, verticalSpacing: 3) {
            GridRow {
                ForEach(columns, id: \.self) { col in
                    Text(col)
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            Divider().gridCellColumns(columns.count)
            ForEach(Array(records.enumerated()), id: \.offset) { _, r in
                GridRow {
                    ForEach(columns, id: \.self) { col in
                        Text(renderValue(r.fields[col]))
                            .font(.system(size: 11, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }

    private func renderValue(_ value: TypedValue?) -> String {
        guard let value else { return "—" }
        return Self.stringify(value)
    }

    private static func stringify(_ value: TypedValue) -> String {
        switch value {
        case .null:           return "null"
        case .bool(let b):    return String(b)
        case .int(let i):     return String(i)
        case .double(let d):  return String(d)
        case .string(let s):  return s
        case .array(let xs):  return "[\(xs.map(stringify).joined(separator: ", "))]"
        case .record(let r):  return "{\(r.typeName)}"
        }
    }

    private struct TypeCount: Hashable {
        let typeName: String
        let count: Int
    }

    private func countsByType(_ records: [ScrapedRecord]) -> [TypeCount] {
        var c: [String: Int] = [:]
        for r in records { c[r.typeName, default: 0] += 1 }
        return c.map { TypeCount(typeName: $0.key, count: $0.value) }
            .sorted { $0.typeName < $1.typeName }
    }
}
