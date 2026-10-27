import SwiftUI
import WebKit

// A thin shell: it opens the web App Hub (docs/web-hub.md) full screen. All
// Plinth logic runs in the page, so this file has no app code. Replace it
// with the gpui host when that exists (docs/MAC-HANDOFF.md, track B).

@main
struct PlinthHubApp: App {
    var body: some Scene {
        WindowGroup {
            HubWebView(url: hubURL)
                .ignoresSafeArea(.container, edges: .bottom)
        }
    }

    private var hubURL: URL {
        let text = Bundle.main.object(forInfoDictionaryKey: "PlinthHubURL") as? String
        return URL(string: text ?? "https://plinth.femur.dev/")!
    }
}

struct HubWebView: UIViewRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        // The default store keeps OPFS, localStorage and IndexedDB between
        // launches: the Hub library and the app data live there.
        config.websiteDataStore = .default()
        config.allowsInlineMediaPlayback = true
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = false
        #if DEBUG
        view.isInspectable = true // Safari > Develop > [device] on the Mac
        #endif
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        // A link to another site opens in Safari, not inside the shell.
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            guard let target = action.request.url, action.targetFrame?.isMainFrame ?? true,
                  let host = webView.url?.host, let to = target.host, host != to, target.scheme?.hasPrefix("http") == true
            else { return decisionHandler(.allow) }
            UIApplication.shared.open(target)
            decisionHandler(.cancel)
        }

        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                     for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = action.request.url { UIApplication.shared.open(url) }
            return nil
        }
    }
}
