import Foundation

/// Talks to `api.foragelang.com` (or a configured override). M3 ships a
/// stub-mode flow for `publish` — instead of actually POSTing, it returns
/// the "would POST" preview. `get` / `list` perform real HTTPS requests if
/// the configured hub is reachable; otherwise they throw a friendly error
/// the caller can surface in the UI.
struct HubClient: Sendable {
    let baseURL: String
    let apiKey: String?

    init(baseURL: String, apiKey: String?) {
        self.baseURL = baseURL
        self.apiKey = apiKey
    }

    func health() async throws -> String {
        try await getString(path: "/v1/health")
    }

    func list() async throws -> String {
        try await getString(path: "/v1/recipes")
    }

    func get(slug: String) async throws -> String {
        try await getString(path: "/v1/recipes/\(slug)")
    }

    /// M3: stub. Validates that we have a base URL + key, but doesn't
    /// actually POST. Returns a human-readable summary the UI can dump
    /// into the publish-output panel. M4 will swap in a real
    /// `URLSession.shared.data(for:)` call.
    ///
    /// Takes the payload as pre-encoded JSON `Data` so the call site can
    /// build a `[String: Any]` dictionary on the main actor without having
    /// to make it Sendable.
    func publish(payloadJSON: Data) async throws -> String {
        guard !baseURL.isEmpty, URL(string: baseURL) != nil else {
            throw HubError.invalidBaseURL(baseURL)
        }
        guard let apiKey, !apiKey.isEmpty else {
            throw HubError.missingAPIKey
        }
        let endpoint = "\(baseURL.trimmingCharacters(in: CharacterSet(charactersIn: "/")))/v1/recipes"
        let pretty = String(data: payloadJSON, encoding: .utf8) ?? "{}"
        let keyHint = String(apiKey.prefix(4)) + "…"
        return [
            "[M3 stub — not actually POSTing yet; M4 wires this live]",
            "Would POST to \(endpoint)",
            "Authorization: Bearer \(keyHint)",
            "",
            "Body:",
            pretty,
        ].joined(separator: "\n")
    }

    // MARK: - Helpers

    private func getString(path: String) async throws -> String {
        guard let url = URL(string: baseURL.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + path) else {
            throw HubError.invalidBaseURL(baseURL)
        }
        var request = URLRequest(url: url)
        if let apiKey, !apiKey.isEmpty {
            request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        }
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw HubError.nonHTTPResponse
            }
            let body = String(data: data, encoding: .utf8) ?? ""
            if (200..<300).contains(http.statusCode) {
                return body
            } else {
                throw HubError.badStatus(http.statusCode, body)
            }
        } catch let error as URLError {
            throw HubError.networkUnreachable(error.localizedDescription)
        }
    }
}

enum HubError: Error, CustomStringConvertible {
    case invalidBaseURL(String)
    case missingAPIKey
    case nonHTTPResponse
    case badStatus(Int, String)
    case networkUnreachable(String)

    var description: String {
        switch self {
        case .invalidBaseURL(let s):
            return "Hub URL isn't a valid URL: \(s)"
        case .missingAPIKey:
            return "Configure your API key in Preferences (Cmd-,) before publishing."
        case .nonHTTPResponse:
            return "Got a non-HTTP response from the hub."
        case .badStatus(let code, let body):
            return "Hub returned \(code): \(body)"
        case .networkUnreachable(let s):
            return "Couldn't reach the hub: \(s)"
        }
    }
}
