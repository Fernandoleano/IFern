use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Flex, Hoverable, MainAxisSize, MouseStateHandle, Padding, ParentElement, Radius, Shrinkable,
    Text,
};
use warpui::event::DispatchedEvent;
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{
    AfterLayoutContext, AppContext, Entity, EntityId, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    PropagateHorizontalNavigationKeys, SingleLineEditorOptions, TextOptions,
};

#[cfg(target_os = "macos")]
mod native {
    use std::sync::Once;

    use cocoa::base::{id, nil, BOOL, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::rc::StrongPtr;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    static OWNER_IVAR: &str = "_warp_browser_owner_web_view";

    /// Returns the registered Objective-C class that acts as the WKWebView's
    /// `UIDelegate`. The delegate redirects popup-window creation
    /// (`window.open`, target=_blank) into the host WKWebView so flows like
    /// "Sign in with Google" work in-place instead of silently failing.
    fn ui_delegate_class() -> &'static Class {
        static INIT: Once = Once::new();
        static mut DELEGATE: *const Class = std::ptr::null();
        unsafe {
            INIT.call_once(|| {
                let superclass = class!(NSObject);
                let mut decl = ClassDecl::new("WarpBrowserUIDelegate", superclass)
                    .expect("WarpBrowserUIDelegate already registered");
                decl.add_ivar::<id>(OWNER_IVAR);

                extern "C" fn create_web_view(
                    this: &mut Object,
                    _sel: Sel,
                    _web_view: id,
                    _configuration: id,
                    navigation_action: id,
                    _window_features: id,
                ) -> id {
                    unsafe {
                        let request: id = msg_send![navigation_action, request];
                        let owner: id = *this.get_ivar(OWNER_IVAR);
                        if request != nil && owner != nil {
                            let _: id = msg_send![owner, loadRequest: request];
                        }
                    }
                    nil
                }

                decl.add_method(
                    sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
                    create_web_view
                        as extern "C" fn(&mut Object, Sel, id, id, id, id) -> id,
                );

                DELEGATE = decl.register();
            });
            &*DELEGATE
        }
    }

    /// Owns a native WKWebView added as a subview of a parent NSView. The
    /// WKWebView is removed from its superview when this struct is dropped.
    pub struct NativeWebView {
        web_view: StrongPtr,
        ui_delegate: StrongPtr,
    }

    impl NativeWebView {
        /// Creates a WKWebView with the given frame (in AppKit point coordinates,
        /// flipped Y) and adds it as a subview of `parent_view`.
        pub fn new(parent_view: id, frame: NSRect) -> Option<Self> {
            unsafe {
                let configuration: id = msg_send![class!(WKWebViewConfiguration), new];
                if configuration == nil {
                    return None;
                }

                // Allow inline media so embedded video players (YouTube, etc.)
                // work without forcing fullscreen.
                let _: () = msg_send![configuration, setAllowsAirPlayForMediaPlayback: YES];

                let alloc: id = msg_send![class!(WKWebView), alloc];
                if alloc == nil {
                    let _: () = msg_send![configuration, release];
                    return None;
                }
                let web_view: id = msg_send![alloc, initWithFrame: frame configuration: configuration];
                let _: () = msg_send![configuration, release];
                if web_view == nil {
                    return None;
                }

                // Match parent on resize so a cosmetic gap doesn't appear if our
                // explicit setFrame: lags by a frame.
                // NSViewWidthSizable | NSViewHeightSizable
                let autoresizing: u64 = (1 << 1) | (1 << 4);
                let _: () = msg_send![web_view, setAutoresizingMask: autoresizing];

                // Two-finger swipe back/forward for trackpads.
                let _: () = msg_send![web_view, setAllowsBackForwardNavigationGestures: YES];
                let _: () = msg_send![web_view, setAllowsMagnification: YES];

                // Mimic Safari so OAuth providers (Google, Microsoft, etc.)
                // don't reject the request as coming from an embedded browser.
                let ua = NSString::alloc(nil).init_str(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                     AppleWebKit/605.1.15 (KHTML, like Gecko) \
                     Version/17.0 Safari/605.1.15",
                );
                let _: () = msg_send![web_view, setCustomUserAgent: ua];
                let _: () = msg_send![ua, release];

                // Install the popup-redirect UIDelegate. We retain the delegate
                // alongside the web view because WKWebView holds a weak ref.
                let delegate: id = msg_send![ui_delegate_class(), new];
                if delegate != nil {
                    (*delegate).set_ivar(OWNER_IVAR, web_view);
                    let _: () = msg_send![web_view, setUIDelegate: delegate];
                }
                let ui_delegate = StrongPtr::new(delegate);

                let _: () = msg_send![parent_view, addSubview: web_view];

                Some(Self {
                    web_view: StrongPtr::new(web_view),
                    ui_delegate,
                })
            }
        }

        pub fn load_url(&self, url: &str) {
            unsafe {
                let url_str: id = NSString::alloc(nil).init_str(url);
                let ns_url: id = msg_send![class!(NSURL), URLWithString: url_str];
                let _: () = msg_send![url_str, release];
                if ns_url == nil {
                    return;
                }
                let request: id = msg_send![class!(NSURLRequest), requestWithURL: ns_url];
                let _: id = msg_send![*self.web_view, loadRequest: request];
            }
        }

        /// Make the WKWebView the window's first responder so it receives
        /// keyboard input.
        pub fn focus(&self) {
            unsafe {
                let window: id = msg_send![*self.web_view, window];
                if window != nil {
                    let _: BOOL = msg_send![window, makeFirstResponder: *self.web_view];
                }
            }
        }

        pub fn set_frame(&self, frame: NSRect) {
            unsafe {
                let _: () = msg_send![*self.web_view, setFrame: frame];
            }
        }

        pub fn set_hidden(&self, hidden: bool) {
            let value: BOOL = if hidden { YES } else { NO };
            unsafe {
                let _: () = msg_send![*self.web_view, setHidden: value];
            }
        }

        pub fn go_back(&self) {
            unsafe {
                let _: id = msg_send![*self.web_view, goBack];
            }
        }

        pub fn go_forward(&self) {
            unsafe {
                let _: id = msg_send![*self.web_view, goForward];
            }
        }

        pub fn reload(&self) {
            unsafe {
                let _: id = msg_send![*self.web_view, reload];
            }
        }

        pub fn can_go_back(&self) -> bool {
            unsafe {
                let b: BOOL = msg_send![*self.web_view, canGoBack];
                b == YES
            }
        }

        pub fn can_go_forward(&self) -> bool {
            unsafe {
                let b: BOOL = msg_send![*self.web_view, canGoForward];
                b == YES
            }
        }
    }

    impl Drop for NativeWebView {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![*self.web_view, removeFromSuperview];
            }
        }
    }

    pub fn make_rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    /// Resolve the active window's content NSView via the warpui mac platform
    /// extension.
    pub fn active_window_content_view(app: &super::AppContext) -> Option<id> {
        use warpui::platform::mac::WindowExt;
        let window_id = app.windows().active_window()?;
        let platform_window = app.windows().platform_window(window_id)?;
        let dyn_window: &dyn warpui::platform::Window = platform_window.as_ref();
        (&dyn_window).native_content_view()
    }
}

#[cfg(target_os = "macos")]
use native::NativeWebView;

/// Empty element that claims its full available constraint and saves its
/// painted bounds to the position cache. Used to mark the rect that the
/// embedded WKWebView should overlay.
struct WebViewArea {
    position_id: String,
    size: Option<Vector2F>,
}

impl WebViewArea {
    fn new(position_id: String) -> Box<Self> {
        Box::new(Self {
            position_id,
            size: None,
        })
    }
}

impl Element for WebViewArea {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        // Fill all available space. Width comes from cross-axis stretch in the
        // parent column; height comes from the Shrinkable flex factor.
        let width = if constraint.max.x().is_finite() {
            constraint.max.x()
        } else {
            constraint.min.x()
        };
        let height = if constraint.max.y().is_finite() {
            constraint.max.y()
        } else {
            constraint.min.y()
        };
        let size = Vector2F::new(width.max(0.0), height.max(0.0));
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        if let Some(size) = self.size {
            ctx.position_cache
                .cache_position_indefinitely(self.position_id.clone(), RectF::new(origin, size));
        }
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<warpui::elements::Point> {
        None
    }
}

const DEFAULT_URL: &str = "https://www.warp.dev";

/// Chrome-style omnibox: decide whether the user typed a URL or a search query.
/// Mirrors how Chrome and Safari handle ambiguous input — anything without a
/// dot (and that isn't `localhost` or an IP-ish thing) is treated as a search
/// and routed through Google.
fn resolve_omnibox_input(input: &str) -> String {
    let trimmed = input.trim();

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    if trimmed.starts_with("about:") || trimmed.starts_with("file://") {
        return trimmed.to_string();
    }

    let host_part = trimmed.split('/').next().unwrap_or("");
    let looks_like_host = host_part == "localhost"
        || host_part.starts_with("localhost:")
        || (host_part.contains('.')
            && !host_part.contains(' ')
            && host_part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_')));

    if looks_like_host {
        format!("https://{trimmed}")
    } else {
        format!(
            "https://www.google.com/search?q={}",
            urlencoding::encode(trimmed)
        )
    }
}

#[cfg(test)]
mod resolve_omnibox_input_tests {
    use super::resolve_omnibox_input;

    #[test]
    fn keeps_full_urls() {
        assert_eq!(
            resolve_omnibox_input("https://example.com/foo"),
            "https://example.com/foo"
        );
        assert_eq!(
            resolve_omnibox_input("http://example.com"),
            "http://example.com"
        );
    }

    #[test]
    fn prefixes_bare_hosts_with_https() {
        assert_eq!(resolve_omnibox_input("youtube.com"), "https://youtube.com");
        assert_eq!(
            resolve_omnibox_input("localhost:3000"),
            "https://localhost:3000"
        );
    }

    #[test]
    fn searches_when_no_dot() {
        assert_eq!(
            resolve_omnibox_input("rust async runtime"),
            "https://www.google.com/search?q=rust%20async%20runtime"
        );
        assert_eq!(
            resolve_omnibox_input("youtube"),
            "https://www.google.com/search?q=youtube"
        );
    }
}

#[derive(Clone, Debug)]
pub enum BrowserViewAction {
    Navigate,
    Back,
    Forward,
    Reload,
}

pub struct BrowserView {
    url_editor: ViewHandle<EditorView>,
    current_url: String,
    state_handles: BrowserStateHandles,
    #[cfg(target_os = "macos")]
    web_view: RefCell<Option<NativeWebView>>,
    /// Whether the panel is currently the active left-panel view. Updated from
    /// outside (LeftPanelView) so the WKWebView can be hidden when the panel is
    /// not visible — render() doesn't run in that case.
    panel_visible: Rc<Cell<bool>>,
    self_id_seed: OnceCell<EntityId>,
}

#[derive(Default)]
struct BrowserStateHandles {
    back_button: MouseStateHandle,
    forward_button: MouseStateHandle,
    reload_button: MouseStateHandle,
    go_button: MouseStateHandle,
}

impl BrowserView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let url_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(13.), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: false,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_horizontal_navigation_keys:
                        PropagateHorizontalNavigationKeys::AtBoundary,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("Enter URL...", ctx);
            editor.set_buffer_text(DEFAULT_URL, ctx);
            editor
        });

        ctx.subscribe_to_view(&url_editor, |me, _handle, event, ctx| {
            if let EditorEvent::Enter = event {
                me.handle_navigate(ctx);
            }
        });

        Self {
            url_editor,
            current_url: DEFAULT_URL.to_string(),
            state_handles: BrowserStateHandles::default(),
            #[cfg(target_os = "macos")]
            web_view: RefCell::new(None),
            panel_visible: Rc::new(Cell::new(false)),
            self_id_seed: OnceCell::new(),
        }
    }

    fn position_id(&self, _ctx: &AppContext) -> String {
        // ViewContext isn't available in render(); use a stable per-instance id
        // derived from the editor's id (a one-time seed).
        let seed = self.self_id_seed.get_or_init(|| self.url_editor.id());
        format!("browser_panel_content_{seed}")
    }

    fn handle_navigate(&mut self, ctx: &mut ViewContext<Self>) {
        let raw = self.url_editor.as_ref(ctx).buffer_text(ctx);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let url = resolve_omnibox_input(trimmed);
        self.current_url = url.clone();

        #[cfg(target_os = "macos")]
        {
            self.ensure_web_view(ctx);
            if let Some(wv) = self.web_view.borrow().as_ref() {
                wv.load_url(&url);
            }
        }
        ctx.notify();
    }

    #[cfg(target_os = "macos")]
    fn ensure_web_view(&self, ctx: &AppContext) {
        if self.web_view.borrow().is_some() {
            return;
        }
        match native::active_window_content_view(ctx) {
            Some(parent) => {
                log::warn!("[browser] creating WKWebView, parent={:p}", parent);
                let frame = native::make_rect(0.0, 0.0, 1.0, 1.0);
                if let Some(wv) = NativeWebView::new(parent, frame) {
                    wv.set_hidden(true);
                    wv.load_url(&self.current_url);
                    *self.web_view.borrow_mut() = Some(wv);
                    log::warn!("[browser] WKWebView created");
                } else {
                    log::warn!("[browser] WKWebView::new returned None");
                }
            }
            None => {
                log::warn!("[browser] could not get content view from app");
            }
        }
    }

    pub fn on_panel_focused(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.url_editor);
    }

    /// Called by the parent (LeftPanelView) whenever the active left-panel view
    /// changes. Controls visibility of the embedded WKWebView, which lives in
    /// the native view hierarchy and is independent of warpui rendering.
    pub fn set_panel_visible(&self, visible: bool) {
        log::warn!("[browser] set_panel_visible({visible})");
        self.panel_visible.set(visible);
        #[cfg(target_os = "macos")]
        {
            if !visible {
                if let Some(wv) = self.web_view.borrow().as_ref() {
                    wv.set_hidden(true);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn update_web_view_frame(&self, app: &AppContext) {
        let panel_visible = self.panel_visible.get();
        log::warn!("[browser] update_web_view_frame visible={panel_visible}");
        if !panel_visible {
            if let Some(wv) = self.web_view.borrow().as_ref() {
                wv.set_hidden(true);
            }
            return;
        }

        let Some(window_id) = app.windows().active_window() else {
            log::warn!("[browser] no active window");
            return;
        };
        let Some(platform_window) = app.windows().platform_window(window_id) else {
            log::warn!("[browser] no platform window");
            return;
        };
        let window_size = platform_window.size();
        let window_height = window_size.y() as f64;

        let pid = self.position_id(app);
        let panel_rect = app.element_position_by_id_at_last_frame(window_id, &pid);
        log::warn!(
            "[browser] window_size=({}, {}) pid={pid} panel_rect={panel_rect:?}",
            window_size.x(),
            window_size.y(),
        );

        let Some(panel_rect) = panel_rect else {
            self.ensure_web_view(app);
            if let Some(wv) = self.web_view.borrow().as_ref() {
                wv.set_hidden(true);
            }
            return;
        };

        self.ensure_web_view(app);

        let x = panel_rect.origin_x() as f64;
        let y_warpui = panel_rect.origin_y() as f64;
        let width = panel_rect.width() as f64;
        let height = panel_rect.height() as f64;
        let y_appkit = window_height - y_warpui - height;
        let frame = native::make_rect(x, y_appkit, width.max(0.0), height.max(0.0));
        log::warn!(
            "[browser] applying frame x={x} y_appkit={y_appkit} w={width} h={height}"
        );

        if let Some(wv) = self.web_view.borrow().as_ref() {
            wv.set_frame(frame);
            wv.set_hidden(false);
        } else {
            log::warn!("[browser] web_view is None after ensure_web_view");
        }
    }

    fn can_go_back(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.web_view
                .borrow()
                .as_ref()
                .map(|wv| wv.can_go_back())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    fn can_go_forward(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.web_view
                .borrow()
                .as_ref()
                .map(|wv| wv.can_go_forward())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

fn render_nav_button(
    icon: Icon,
    enabled: bool,
    mouse_state: MouseStateHandle,
    action: BrowserViewAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_color = if enabled {
        theme.main_text_color(theme.background())
    } else {
        theme.disabled_ui_text_color()
    };

    let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
        .with_width(16.)
        .with_height(16.)
        .finish();

    let button = Hoverable::new(mouse_state, move |mouse_state| {
        let mut container = Container::new(icon_el)
            .with_padding(Padding::uniform(4.))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if enabled && mouse_state.is_hovered() {
            container = container.with_background(theme.surface_3());
        }
        container.finish()
    });

    if enabled {
        button
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .finish()
    } else {
        button.finish()
    }
}

impl Entity for BrowserView {
    type Event = ();
}

impl TypedActionView for BrowserView {
    type Action = BrowserViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BrowserViewAction::Navigate => {
                self.handle_navigate(ctx);
            }
            BrowserViewAction::Back => {
                #[cfg(target_os = "macos")]
                {
                    if let Some(wv) = self.web_view.borrow().as_ref() {
                        wv.go_back();
                    }
                }
                ctx.notify();
            }
            BrowserViewAction::Forward => {
                #[cfg(target_os = "macos")]
                {
                    if let Some(wv) = self.web_view.borrow().as_ref() {
                        wv.go_forward();
                    }
                }
                ctx.notify();
            }
            BrowserViewAction::Reload => {
                #[cfg(target_os = "macos")]
                {
                    if let Some(wv) = self.web_view.borrow().as_ref() {
                        wv.reload();
                    }
                }
                ctx.notify();
            }
        }
    }
}

impl View for BrowserView {
    fn ui_name() -> &'static str {
        "BrowserView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Update the embedded WKWebView frame and visibility based on the
        // last-frame cached position of this panel's content area.
        #[cfg(target_os = "macos")]
        self.update_web_view_frame(app);

        // Navigation bar: [Back] [Forward] [Reload] [URL bar] [Go]
        let back = render_nav_button(
            Icon::ArrowLeft,
            self.can_go_back(),
            self.state_handles.back_button.clone(),
            BrowserViewAction::Back,
            appearance,
        );
        let forward = render_nav_button(
            Icon::ArrowRight,
            self.can_go_forward(),
            self.state_handles.forward_button.clone(),
            BrowserViewAction::Forward,
            appearance,
        );
        let reload = render_nav_button(
            Icon::Refresh,
            true,
            self.state_handles.reload_button.clone(),
            BrowserViewAction::Reload,
            appearance,
        );

        let url_input = Shrinkable::new(
            1.0,
            Container::new(ChildView::new(&self.url_editor).finish())
                .with_padding(Padding::uniform(4.).with_left(8.).with_right(8.))
                .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
        )
        .finish();

        let go_button = render_nav_button(
            Icon::ArrowRight,
            true,
            self.state_handles.go_button.clone(),
            BrowserViewAction::Navigate,
            appearance,
        );

        let nav_bar = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.)
                .with_child(back)
                .with_child(forward)
                .with_child(reload)
                .with_child(url_input)
                .with_child(go_button)
                .finish(),
        )
        .with_padding(Padding::uniform(6.).with_left(8.).with_right(8.))
        .finish();

        // Title
        let title_section = Container::new(
            Text::new("Browser", appearance.ui_font_family(), 14.)
                .with_color(theme.main_text_color(theme.background()).into_solid())
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        )
        .with_padding(Padding::uniform(0.).with_left(12.).with_right(12.).with_top(8.))
        .finish();

        // Content area: a custom element that claims its full constraint and
        // saves its painted bounds so the embedded WKWebView can overlay the
        // exact rect on the next frame.
        let content_placeholder = WebViewArea::new(self.position_id(app));

        // Assemble full layout
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(nav_bar)
            .with_child(title_section)
            .with_child(Shrinkable::new(1.0, content_placeholder).finish())
            .finish()
    }
}

