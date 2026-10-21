//! Windows hizmet yonetimi: kurulum, kaldirma, baslangic turu ve hizmet izinleri.
//!
//! Hizmet izinleri, etkilesimli kullanicilarin (IU) hizmeti baslatip durdurabilmesine
//! izin verir: boylece "El ile" baslangicta arayuz UAC sormadan motoru acabilir.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::System::Services::{
    ChangeServiceConfigW, CloseServiceHandle, ENUM_SERVICE_TYPE, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT,
    SERVICE_AUTO_START, SERVICE_CHANGE_CONFIG, SERVICE_DEMAND_START, SERVICE_ERROR, SERVICE_NO_CHANGE, SERVICE_START_TYPE,
    SetServiceObjectSecurity,
};
use windows_sys::Win32::Storage::FileSystem::WRITE_DAC;

use shared::{SERVICE_NAME, t};

// SYSTEM ve Yoneticiler tam; Etkilesimli kullanicilar: sorgula, baslat, durdur, oku.
const SERVICE_SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPWPLORC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn install(exe: &Path, auto_start: bool) -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| format!("Hizmet yöneticisine bağlanılamadı (yönetici gerekli): {e}"))?;

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(t!("DPI UI Motoru", "DPI UI Engine")),
        service_type: ServiceType::OWN_PROCESS,
        start_type: if auto_start { ServiceStartType::AutoStart } else { ServiceStartType::OnDemand },
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_path_buf(),
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::STOP;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(existing) => {
            // Guncelleme: yolu ve ayarlari tazele.
            existing.change_config(&info).map_err(|e| format!("Hizmet güncellenemedi: {e}"))?;
            existing
        }
        Err(_) => manager.create_service(&info, access).map_err(|e| format!("Hizmet oluşturulamadı: {e}"))?,
    };
    let _ = service.set_description(t!(
        "DPI atlatma motoru. Arayüz kapalıyken de bağlantıyı korur; çok az kaynak kullanır.",
        "DPI bypass engine. Keeps the connection protected even when the app is closed; uses very few resources."
    ));

    // Cokme ya da beklenmedik cikista 5 / 10 / 30 sn sonra yeniden baslat.
    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(10) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(30) },
        ]),
    };
    let _ = service.update_failure_actions(actions);
    let _ = service.set_failure_actions_on_non_crash_failures(true);
    drop(service);

    set_security()?;
    Ok(())
}

fn set_security() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err("Hizmet yöneticisi açılamadı.".into());
        }
        let svc = OpenServiceW(scm, wide(SERVICE_NAME).as_ptr(), WRITE_DAC);
        let mut sd = std::ptr::null_mut();
        let ok = !svc.is_null()
            && ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(SERVICE_SDDL).as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut())
                != 0
            && SetServiceObjectSecurity(svc, DACL_SECURITY_INFORMATION, sd) != 0;
        if !sd.is_null() {
            windows_sys::Win32::Foundation::LocalFree(sd as _);
        }
        if !svc.is_null() {
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
        if ok { Ok(()) } else { Err("Hizmet izinleri ayarlanamadı.".into()) }
    }
}

pub fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("Hizmet yöneticisine bağlanılamadı: {e}"))?;
    if let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        stop_and_wait(&service);
        service.delete().map_err(|e| format!("Hizmet silinemedi: {e}"))?;
    }
    // WinDivert surucusunu de birak (baska bir arac kullanmiyorsa durur; kullaniyorsa dokunulmaz).
    if let Ok(driver) = manager.open_service("WinDivert", ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) {
        let _ = driver.stop();
    }
    Ok(())
}

fn stop_and_wait(service: &windows_service::service::Service) {
    let _ = service.stop();
    for _ in 0..50 {
        match service.query_status() {
            Ok(s) if s.current_state == ServiceState::Stopped => return,
            Ok(_) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return,
        }
    }
}

/// Hizmetin kendi baslangic turunu degistirir (hizmet SYSTEM olarak calistigindan izinlidir).
pub fn set_auto_start(auto: bool) -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err("Hizmet yöneticisi açılamadı.".into());
        }
        let svc = OpenServiceW(scm, wide(SERVICE_NAME).as_ptr(), SERVICE_CHANGE_CONFIG);
        let start: SERVICE_START_TYPE = if auto { SERVICE_AUTO_START } else { SERVICE_DEMAND_START };
        let ok = !svc.is_null()
            && ChangeServiceConfigW(
                svc,
                SERVICE_NO_CHANGE as ENUM_SERVICE_TYPE,
                start,
                SERVICE_NO_CHANGE as SERVICE_ERROR,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ) != 0;
        if !svc.is_null() {
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
        if ok { Ok(()) } else { Err("Başlangıç türü değiştirilemedi.".into()) }
    }
}
