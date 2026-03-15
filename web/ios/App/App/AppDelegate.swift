import UIKit
import Capacitor

@UIApplicationMain
class AppDelegate: UIResponder, UIApplicationDelegate {

    var window: UIWindow?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        // Override point for customization after application launch.
        return true
    }

    func applicationWillResignActive(_ application: UIApplication) {
        // Sent when the application is about to move from active to inactive state. This can occur for certain types of temporary interruptions (such as an incoming phone call or SMS message) or when the user quits the application and it begins the transition to the background state.
        // Use this method to pause ongoing tasks, disable timers, and invalidate graphics rendering callbacks. Games should use this method to pause the game.
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
        // Use this method to release shared resources, save user data, invalidate timers, and store enough application state information to restore your application to its current state in case it is terminated later.
        // If your application supports background execution, this method is called instead of applicationWillTerminate: when the user quits.
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
        // Called as part of the transition from the background to the active state; here you can undo many of the changes made on entering the background.
    }

    func applicationDidBecomeActive(_ application: UIApplication) {
        // Restart any tasks that were paused (or not yet started) while the application was inactive. If the application was previously in the background, optionally refresh the user interface.
    }

    func applicationWillTerminate(_ application: UIApplication) {
        // Called when the application is about to terminate. Save data if appropriate. See also applicationDidEnterBackground:.
    }

    func application(_ app: UIApplication, open url: URL, options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
        // Called when the app was launched with a url. Feel free to add additional processing here,
        // but if you want the App API to support tracking app url opens, make sure to keep this call
        return ApplicationDelegateProxy.shared.application(app, open: url, options: options)
    }

    func application(_ application: UIApplication, continue userActivity: NSUserActivity, restorationHandler: @escaping ([UIUserActivityRestoring]?) -> Void) -> Bool {
        // Called when the app was launched with an activity, including Universal Links.
        // Feel free to add additional processing here, but if you want the App API to support
        // tracking app url opens, make sure to keep this call
        return ApplicationDelegateProxy.shared.application(application, continue: userActivity, restorationHandler: restorationHandler)
    }

}

class ThrongtermBridgeViewController: CAPBridgeViewController {
    private static let hostOverrideDefaultsKey = "throngterm.hostOverrideServerURL"

    private let refreshControl = UIRefreshControl()
    private let hostButton = UIButton(type: .system)
    private let openInSafariButton = UIButton(type: .system)
    private var loadingObservation: NSKeyValueObservation?
    private var urlObservation: NSKeyValueObservation?
    private var configuredServerURLString: String?

    override func instanceDescriptor() -> InstanceDescriptor {
        let descriptor = super.instanceDescriptor()
        configuredServerURLString = normalizedServerURLString(descriptor.serverURL)
        descriptor.allowedNavigationHostnames = ["*"]

        if let override = Self.savedHostOverride() {
            descriptor.serverURL = override
        }

        return descriptor
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        configurePullToRefresh()
        configureHostButton()
        configureOpenInSafariButton()
        observeWebViewState()
    }

    deinit {
        loadingObservation?.invalidate()
        urlObservation?.invalidate()
    }

    private func configurePullToRefresh() {
        refreshControl.addTarget(self, action: #selector(handlePullToRefresh), for: .valueChanged)
        webView?.scrollView.refreshControl = refreshControl
    }

    private func configureHostButton() {
        hostButton.setTitle("Host", for: .normal)
        hostButton.titleLabel?.font = UIFont.systemFont(ofSize: 13, weight: .semibold)
        hostButton.backgroundColor = UIColor.systemBackground.withAlphaComponent(0.92)
        hostButton.setTitleColor(.label, for: .normal)
        hostButton.layer.cornerRadius = 14
        hostButton.layer.borderWidth = 1
        hostButton.layer.borderColor = UIColor.separator.withAlphaComponent(0.45).cgColor
        hostButton.contentEdgeInsets = UIEdgeInsets(top: 8, left: 12, bottom: 8, right: 12)
        hostButton.translatesAutoresizingMaskIntoConstraints = false
        hostButton.addTarget(self, action: #selector(editHost), for: .touchUpInside)
        view.addSubview(hostButton)

        NSLayoutConstraint.activate([
            hostButton.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            hostButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
        ])
    }

    private func configureOpenInSafariButton() {
        openInSafariButton.setTitle("Open in Safari", for: .normal)
        openInSafariButton.titleLabel?.font = UIFont.systemFont(ofSize: 13, weight: .semibold)
        openInSafariButton.backgroundColor = UIColor.systemBackground.withAlphaComponent(0.92)
        openInSafariButton.setTitleColor(.label, for: .normal)
        openInSafariButton.layer.cornerRadius = 14
        openInSafariButton.layer.borderWidth = 1
        openInSafariButton.layer.borderColor = UIColor.separator.withAlphaComponent(0.45).cgColor
        openInSafariButton.contentEdgeInsets = UIEdgeInsets(top: 8, left: 12, bottom: 8, right: 12)
        openInSafariButton.translatesAutoresizingMaskIntoConstraints = false
        openInSafariButton.isHidden = true
        openInSafariButton.addTarget(self, action: #selector(openHostInSafari), for: .touchUpInside)
        view.addSubview(openInSafariButton)

        NSLayoutConstraint.activate([
            openInSafariButton.topAnchor.constraint(equalTo: hostButton.bottomAnchor, constant: 8),
            openInSafariButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
        ])
    }

    private func observeWebViewState() {
        guard let webView else {
            return
        }

        loadingObservation = webView.observe(\.isLoading, options: [.new]) { [weak self] _, change in
            guard let self else { return }
            if change.newValue == false {
                self.refreshControl.endRefreshing()
            }
        }

        urlObservation = webView.observe(\.url, options: [.new]) { [weak self] webView, _ in
            guard let self else { return }
            let isErrorPage = webView.url?.lastPathComponent == "mobile-error.html"
            self.openInSafariButton.isHidden = !isErrorPage
            self.hostButton.isHidden = self.shouldHideHostButton(for: webView.url)
            self.updateHostButtonSubtitle(webView.url)
        }

        hostButton.isHidden = shouldHideHostButton(for: webView.url)
        updateHostButtonSubtitle(webView.url)
    }

    @objc private func handlePullToRefresh() {
        webView?.reload()
    }

    @objc private func editHost() {
        let alert = UIAlertController(
            title: "Throngterm Host",
            message: "Set the server URL for this iPhone app.",
            preferredStyle: .alert
        )

        alert.addTextField { [weak self] textField in
            textField.placeholder = "http://100.x.y.z:3210"
            textField.keyboardType = .URL
            textField.autocapitalizationType = .none
            textField.autocorrectionType = .no
            textField.clearButtonMode = .whileEditing
            textField.text = self?.currentServerURLString()
        }

        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))

        alert.addAction(UIAlertAction(title: "Use Config Default", style: .destructive, handler: { [weak self] _ in
            self?.resetToConfiguredHost()
        }))

        alert.addAction(UIAlertAction(title: "Save & Reload", style: .default, handler: { [weak self, weak alert] _ in
            guard let self, let rawValue = alert?.textFields?.first?.text else {
                return
            }
            guard let normalized = self.normalizedServerURLString(rawValue) else {
                self.presentHostValidationError()
                return
            }
            Self.saveHostOverride(normalized)
            self.loadServerURL(normalized)
        }))

        present(alert, animated: true)
    }

    @objc private func openHostInSafari() {
        let candidate = webView?.url?.scheme?.hasPrefix("http") == true ? webView?.url : bridge?.config.serverURL
        guard let url = candidate else {
            return
        }
        UIApplication.shared.open(url, options: [:], completionHandler: nil)
    }

    private func resetToConfiguredHost() {
        Self.clearHostOverride()
        guard let configured = normalizedServerURLString(configuredServerURLString) else {
            return
        }
        loadServerURL(configured)
    }

    private func loadServerURL(_ urlString: String) {
        guard let url = URL(string: urlString) else {
            return
        }
        webView?.load(URLRequest(url: url))
    }

    private func currentServerURLString() -> String {
        if let live = webView?.url, live.scheme?.hasPrefix("http") == true {
            return live.absoluteString
        }
        if let override = Self.savedHostOverride() {
            return override
        }
        if let configured = normalizedServerURLString(configuredServerURLString) {
            return configured
        }
        return bridge?.config.serverURL.absoluteString ?? ""
    }

    private func presentHostValidationError() {
        let alert = UIAlertController(
            title: "Invalid URL",
            message: "Use a full URL like http://100.101.123.63:3210",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
    }

    private func updateHostButtonSubtitle(_ url: URL?) {
        let host = url?.host ?? URL(string: currentServerURLString())?.host ?? "Host"
        hostButton.accessibilityLabel = "Host \(host)"
    }

    private func normalizedServerURLString(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let url = URL(string: trimmed),
              let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              url.host != nil else {
            return nil
        }
        if trimmed.hasSuffix("/") {
            return String(trimmed.dropLast())
        }
        return trimmed
    }

    private func shouldHideHostButton(for url: URL?) -> Bool {
        guard let url else { return false }
        if url.lastPathComponent == "mobile-error.html" {
            return false
        }
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let queryItems = components.queryItems else {
            return false
        }
        return queryItems.contains(where: { $0.name == "view" && $0.value == "terminal" })
    }

    private static func savedHostOverride() -> String? {
        UserDefaults.standard.string(forKey: hostOverrideDefaultsKey)
    }

    private static func saveHostOverride(_ url: String) {
        UserDefaults.standard.set(url, forKey: hostOverrideDefaultsKey)
    }

    private static func clearHostOverride() {
        UserDefaults.standard.removeObject(forKey: hostOverrideDefaultsKey)
    }
}
