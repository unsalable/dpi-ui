//! Java ile kopru. Rust -> Java: `io.github.unsalable.dpiui.Bridge` sinifinin static yontemleri (VPN izni,
//! VPN hizmetini baslatma/durdurma, tema, paylasma). Java -> Rust: `io.github.unsalable.dpiui.Native`
//! sinifinin native yontemleri (asagidaki `Java_io_github_unsalable_dpiui_Native_*` islevleri).
//!
//! Java'nin ana is parcacigindan gelen cagrilar motoru beklemez: baglanma VPN hizmetinin yine ana
//! is parcaciginda calismasini bekledigi icin motora giden isler ayri is parcacigina alinir.

use std::os::fd::RawFd;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{JClass, JString, JValue};
use jni::refs::Global;
use jni::sys::{jboolean, jint};
use jni::{Env, EnvUnowned, JavaVM, jni_sig, jni_str};

struct Java {
    vm: JavaVM,
    /// `io.github.unsalable.dpiui.Bridge`. Uygulama siniflari Rust'in actigi is parcaciklarindan `FindClass`
    /// ile bulunamaz (sistem sinif yukleyicisi); bu yuzden Java is parcaciginda bir kez alinir.
    bridge: Global<JClass<'static>>,
}

static JAVA: OnceLock<Java> = OnceLock::new();

/// Bridge'in static yontemini cagirir. Java hazir degilse ya da cagri hata verirse `None`.
fn call<T>(f: impl FnOnce(&mut Env<'_>, &JClass<'_>) -> jni::errors::Result<T>) -> Option<T> {
    let java = JAVA.get()?;
    match java.vm.attach_current_thread(|env| f(env, &java.bridge)) {
        Ok(v) => Some(v),
        Err(e) => {
            macservice::warn!("Java çağrısı başarısız: {e}");
            None
        }
    }
}

// ------------------------------------------------------------------ Rust -> Java

pub fn vpn_permitted() -> bool {
    call(|env, c| env.call_static_method(c, jni_str!("vpnPermitted"), jni_sig!("()Z"), &[])?.z()).unwrap_or(false)
}

static PERMISSION: Mutex<Option<Box<dyn FnOnce(bool) + Send>>> = Mutex::new(None);

/// Sistemin VPN izni penceresini acar; sonuc `done`a arayuz is parcaciginda gelir.
pub fn request_vpn_permission(done: impl FnOnce(bool) + Send + 'static) {
    *PERMISSION.lock().unwrap() = Some(Box::new(done));
    if call(|env, c| env.call_static_method(c, jni_str!("requestVpnPermission"), jni_sig!("()V"), &[]).map(|_| ())).is_none() {
        permission_result(false);
    }
}

fn permission_result(granted: bool) {
    if let Some(done) = PERMISSION.lock().unwrap().take() {
        let _ = slint::invoke_from_event_loop(move || done(granted));
    }
}

/// VPN hizmetinin kurdugu arayuzun dosya tanitici (ya da hata), `Native.onTun*` ile gelir.
struct TunSlot {
    /// `start_vpn` bekliyor. Beklenmeyen (gec gelen) arayuz kapatilir; yoksa VPN acik kalir ama
    /// trafigi tasiyan olmaz.
    waiting: bool,
    result: Option<Result<RawFd, String>>,
}

static TUN: Mutex<TunSlot> = Mutex::new(TunSlot { waiting: false, result: None });
static TUN_READY: Condvar = Condvar::new();
/// VPN izni penceresi acik kalabilir; hizmetin arayuzu kurmasi bundan uzun surmez.
const TUN_WAIT: Duration = Duration::from_secs(15);

/// VPN hizmetini baslatir ve arayuz kurulana kadar bekler. `title`/`text`: kalici bildirim.
pub fn start_vpn(title: &str, text: &str, stop_label: &str) -> Result<RawFd, String> {
    *TUN.lock().unwrap() = TunSlot { waiting: true, result: None };
    let started = call(|env, c| {
        let (t, x, s) = (env.new_string(title)?, env.new_string(text)?, env.new_string(stop_label)?);
        env.call_static_method(
            c,
            jni_str!("startVpn"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V"),
            &[JValue::Object(&t), JValue::Object(&x), JValue::Object(&s)],
        )
        .map(|_| ())
    });
    if started.is_none() {
        TUN.lock().unwrap().waiting = false;
        return Err(shared::t!("VPN hizmeti başlatılamadı.", "Couldn't start the VPN service."));
    }
    let deadline = Instant::now() + TUN_WAIT;
    let mut slot = TUN.lock().unwrap();
    loop {
        if let Some(result) = slot.result.take() {
            slot.waiting = false;
            return result;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            slot.waiting = false;
            drop(slot);
            stop_vpn();
            return Err(shared::t!("VPN arayüzü kurulamadı (zaman aşımı).", "Couldn't set up the VPN interface (timed out)."));
        }
        slot = TUN_READY.wait_timeout(slot, left).unwrap().0;
    }
}

/// Imzasi dogrulanmis APK'yi sistemin paket yukleyicisine verir (macservice::apk::INSTALLER).
pub fn install_apk(path: &std::path::Path, interactive: bool) -> Result<(), String> {
    let error = call(|env, c| {
        let p = env.new_string(path.to_string_lossy())?;
        let r = env.call_static_method(c, jni_str!("installApk"), jni_sig!("(Ljava/lang/String;Z)Ljava/lang/String;"), &[JValue::Object(&p), JValue::Bool(interactive)])?.l()?;
        if r.is_null() {
            return Ok(None);
        }
        let s = JString::cast_local(env, r)?;
        Ok(Some(string(env, &s)))
    });
    match error {
        Some(None) => Ok(()),
        Some(Some(message)) => Err(shared::t!("Güncelleme kurulamadı: {message}", "Couldn't install the update: {message}")),
        None => Err(shared::t!("Paket yükleyicisine ulaşılamadı.", "Couldn't reach the package installer.")),
    }
}

pub fn stop_vpn() {
    call(|env, c| env.call_static_method(c, jni_str!("stopVpn"), jni_sig!("()V"), &[]).map(|_| ()));
}

/// Bagliyken bildirimdeki metin (yontem degisince).
pub fn update_notification(title: &str, text: &str) {
    call(|env, c| {
        let (t, x) = (env.new_string(title)?, env.new_string(text)?);
        env.call_static_method(c, jni_str!("updateNotification"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"), &[JValue::Object(&t), JValue::Object(&x)])
            .map(|_| ())
    });
}

pub fn night_mode() -> bool {
    call(|env, c| env.call_static_method(c, jni_str!("nightMode"), jni_sig!("()Z"), &[])?.z()).unwrap_or(true)
}

pub fn animations_enabled() -> bool {
    call(|env, c| env.call_static_method(c, jni_str!("animationsEnabled"), jni_sig!("()Z"), &[])?.z()).unwrap_or(true)
}

pub fn set_dark_theme(dark: bool) {
    call(|env, c| env.call_static_method(c, jni_str!("setDarkTheme"), jni_sig!("(Z)V"), &[JValue::Bool(dark)]).map(|_| ()));
}

pub fn open_url(url: &str) {
    call(|env, c| {
        let u = env.new_string(url)?;
        env.call_static_method(c, jni_str!("openUrl"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&u)]).map(|_| ())
    });
}

pub fn share_text(subject: &str, text: &str) {
    call(|env, c| {
        let (s, t) = (env.new_string(subject)?, env.new_string(text)?);
        env.call_static_method(c, jni_str!("shareText"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"), &[JValue::Object(&s), JValue::Object(&t)])
            .map(|_| ())
    });
}

// ------------------------------------------------------------------ Java -> Rust

/// Java dizesi; bossa ya da okunamazsa bos dize.
fn string(env: &Env<'_>, s: &JString<'_>) -> String {
    s.try_to_string(env).unwrap_or_default()
}

/// Motora giden isi Java'nin is parcacigini bekletmeden yapar.
fn in_background(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).stack_size(256 * 1024).spawn(f);
}

/// Uygulama sureci acilinca (etkinlik ya da VPN hizmeti) ilk cagri: veri klasoru ve sistem dili.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_init<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    data_dir: JString<'local>,
    language: JString<'local>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        if JAVA.get().is_some() {
            return Ok(());
        }
        let dir = string(env, &data_dir);
        let lang = string(env, &language);
        // SAFETY: Java tarafi bunu Rust is parcaciklari baslamadan, surecin ilk cagrisi olarak yapar.
        unsafe {
            std::env::set_var("GDPI3_DATA_DIR", &dir);
            // shared::i18n "Sistem" dilini LANG'dan okur.
            std::env::set_var("LANG", if lang.starts_with("tr") { "tr_TR.UTF-8" } else { "en_US.UTF-8" });
        }
        // Motor baslayinca ayardaki dil gecerli olur; o zamana kadar (hizli ayar) sistemin dili.
        shared::i18n::set(shared::i18n::Language::System);
        let mut bind = || -> jni::errors::Result<()> {
            let class = env.find_class(jni_str!("io/github/unsalable/dpiui/Bridge"))?;
            let bridge = env.new_global_ref(class)?;
            let vm = env.get_java_vm()?;
            let _ = JAVA.set(Java { vm, bridge });
            Ok(())
        };
        let result = bind();
        // Kurulamazsa VPN, tema ve paylasma calismaz; nedeni gunlukte gorunsun.
        if let Err(e) = &result {
            macservice::error!("Java köprüsü kurulamadı: {e}");
        }
        result
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// Arayuz disindan baglanma: hizli ayar, her zaman acik VPN (`only_if_auto` yanlis) ya da
/// telefon acilisi (dogru: yalnizca "Acilista otomatik baglan" aciksa). Motor gerekirse baslar.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_connect<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>, only_if_auto: jboolean) {
    let only_if_auto: bool = only_if_auto.into();
    in_background("baglan", move || {
        let Some(ctrl) = crate::engine_host::ensure() else {
            stop_vpn();
            return;
        };
        // Motor baslarken ayara gore zaten baglanmis olabilir.
        if ctrl.status().state == shared::ipc::EngineState::On {
            return;
        }
        if only_if_auto && !ctrl.config().auto_connect {
            // Hizmet bildirimle acildi; baglanilmayacaksa kapansin.
            stop_vpn();
            return;
        }
        let _ = ctrl.connect();
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_disconnect<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>) {
    in_background("kes", || {
        if let Some(ctrl) = crate::engine_host::running() {
            ctrl.disconnect();
        }
    });
}

/// Hizli ayar kutucugunun alt yazisi, uygulamanin dilinde.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_statusLabel<'local>(mut env: EnvUnowned<'local>, _class: JClass<'local>, on: jboolean) -> JString<'local> {
    let on: bool = on.into();
    env.with_env(|env| -> jni::errors::Result<JString<'local>> {
        env.new_string(if on { shared::t!("Korumalı", "Protected") } else { shared::t!("Kapalı", "Off") })
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// Hizli ayar kutucugu icin: motor bagli mi? Java'nin ana is parcacigindan gelir; baglanma suruyorsa
/// (motor VPN hizmetini, yani yine ana is parcacigini bekliyor) beklemeden "hayir" der.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_isConnected<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>) -> jboolean {
    crate::engine_host::running().and_then(|c| c.try_status()).is_some_and(|s| s.state == shared::ipc::EngineState::On).into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_onVpnPermission<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>, granted: jboolean) {
    permission_result(granted.into());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_onTunReady<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>, fd: jint) {
    let mut slot = TUN.lock().unwrap();
    if slot.waiting {
        slot.result = Some(Ok(fd));
        TUN_READY.notify_all();
        return;
    }
    drop(slot);
    macservice::warn!("Beklenmeyen VPN arayüzü kapatılıyor.");
    // SAFETY: Java tanitici `detachFd` ile birakti; kullanan yok, burada kapanir.
    drop(unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) });
    stop_vpn();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_onTunFailed<'local>(mut env: EnvUnowned<'local>, _class: JClass<'local>, message: JString<'local>) {
    let message = env.with_env(|env| -> jni::errors::Result<String> { Ok(string(env, &message)) }).resolve::<jni::errors::LogErrorAndDefault>();
    let mut slot = TUN.lock().unwrap();
    if slot.waiting {
        slot.result = Some(Err(message));
        TUN_READY.notify_all();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_onInstallResult<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    ok: jboolean,
    message: JString<'local>,
) {
    let message = env.with_env(|env| -> jni::errors::Result<String> { Ok(string(env, &message)) }).resolve::<jni::errors::LogErrorAndDefault>();
    let ok: bool = ok.into();
    if !ok {
        in_background("guncelleme-sonucu", move || {
            if let Some(ctrl) = crate::engine_host::running() {
                ctrl.install_failed(&message);
            }
        });
    }
}

/// Kullanici VPN'i sistem ayarlarindan kapatti ya da baska bir VPN uygulamasi acildi.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_unsalable_dpiui_Native_onVpnRevoked<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>) {
    in_background("vpn-geri-alindi", || {
        if let Some(ctrl) = crate::engine_host::running() {
            macservice::warn!("VPN sistem tarafından kapatıldı.");
            ctrl.disconnect();
        }
    });
}
