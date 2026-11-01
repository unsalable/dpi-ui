package io.github.unsalable.dpiui;

import android.content.Context;

import java.util.Locale;

/** Rust kutuphanesi (libdpiui.so): motor, VPN aktarimi ve arayuz. */
final class Native {
    private static boolean ready;

    private Native() {}

    /**
     * Kutuphaneyi yukler ve bir kez hazirlar (veri klasoru, sistem dili, Java sinifi). Etkinlik,
     * VPN hizmeti, acilis alicisi ve hizli ayar hangisi once calisirsa o cagirir.
     */
    static synchronized void ensureInit(Context context) {
        if (ready) {
            return;
        }
        Context app = context.getApplicationContext();
        Bridge.app = app;
        System.loadLibrary("dpiui");
        init(app.getFilesDir().getAbsolutePath(), Locale.getDefault().getLanguage());
        ready = true;
    }

    private static native void init(String dataDir, String language);

    /** Baglanir; {@code onlyIfAuto}: yalnizca "Acilista otomatik baglan" aciksa (telefon acilisi). */
    static native void connect(boolean onlyIfAuto);

    static native void disconnect();

    static native boolean isConnected();

    /** "Korumali" / "Kapali", uygulamanin secili dilinde. */
    static native String statusLabel(boolean on);

    static native void onVpnPermission(boolean granted);

    /** VPN arayuzu kuruldu; tanitici artik Rust'in ({@code detachFd}). */
    static native void onTunReady(int fd);

    static native void onTunFailed(String message);

    static native void onVpnRevoked();

    /** Paket yukleyicisi kurmadi (kullanici vazgecti ya da hata). */
    static native void onInstallResult(boolean ok, String message);
}
