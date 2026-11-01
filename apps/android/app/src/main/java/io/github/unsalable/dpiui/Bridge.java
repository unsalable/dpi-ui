package io.github.unsalable.dpiui;

import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.res.Configuration;
import android.net.Uri;
import android.net.VpnService;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;

/**
 * Rust'in cagirdigi Java yontemleri (bridge.rs). Rust'in kendi is parcaciklarindan gelir;
 * arayuze ve hizmete dokunan isler ana is parcacigina aktarilir.
 */
final class Bridge {
    static Context app;
    private static final Handler MAIN = new Handler(Looper.getMainLooper());

    private Bridge() {}

    static boolean vpnPermitted() {
        return VpnService.prepare(app) == null;
    }

    /** Sistemin VPN izni penceresi; sonuc {@link Native#onVpnPermission} ile doner. */
    static void requestVpnPermission() {
        MAIN.post(() -> {
            MainActivity activity = MainActivity.current;
            if (activity == null) {
                Native.onVpnPermission(false);
            } else {
                activity.requestVpnPermission();
            }
        });
    }

    /** VPN hizmetini baslatir; arayuz kurulunca {@link Native#onTunReady} cagrilir. */
    static void startVpn(String title, String text, String stopLabel) {
        Intent intent = new Intent(app, DpiVpnService.class)
                .setAction(DpiVpnService.ACTION_START)
                .putExtra(DpiVpnService.EXTRA_TITLE, title)
                .putExtra(DpiVpnService.EXTRA_TEXT, text)
                .putExtra(DpiVpnService.EXTRA_STOP, stopLabel);
        app.startForegroundService(intent);
    }

    /** Imzasi dogrulanmis APK'yi kurar; hata metni ya da null. */
    static String installApk(String path, boolean interactive) {
        return Updater.install(app, path, interactive);
    }

    static void stopVpn() {
        MAIN.post(() -> {
            DpiVpnService service = DpiVpnService.instance;
            if (service != null) {
                service.shutdown();
            }
        });
    }

    static void updateNotification(String title, String text) {
        MAIN.post(() -> {
            DpiVpnService service = DpiVpnService.instance;
            if (service != null) {
                service.showStatus(title, text);
            }
        });
    }

    static boolean nightMode() {
        int mode = app.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        return mode != Configuration.UI_MODE_NIGHT_NO;
    }

    /** Gelistirici seceneklerinde animasyon olcegi 0 ise gecisler aninda olur. */
    static boolean animationsEnabled() {
        return Settings.Global.getFloat(app.getContentResolver(), Settings.Global.ANIMATOR_DURATION_SCALE, 1f) != 0f;
    }

    static void setDarkTheme(boolean dark) {
        MAIN.post(() -> {
            MainActivity activity = MainActivity.current;
            if (activity != null) {
                activity.applySystemBars(dark);
            }
        });
    }

    static void openUrl(String url) {
        Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            app.startActivity(intent);
        } catch (ActivityNotFoundException ignored) {
            // Tarayici yok.
        }
    }

    static void shareText(String subject, String text) {
        Intent send = new Intent(Intent.ACTION_SEND)
                .setType("text/plain")
                .putExtra(Intent.EXTRA_SUBJECT, subject)
                .putExtra(Intent.EXTRA_TEXT, text);
        Intent chooser = Intent.createChooser(send, subject).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            app.startActivity(chooser);
        } catch (ActivityNotFoundException ignored) {
            // Paylasacak uygulama yok.
        }
    }
}
