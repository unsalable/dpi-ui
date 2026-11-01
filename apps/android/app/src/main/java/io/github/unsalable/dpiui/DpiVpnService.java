package io.github.unsalable.dpiui;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.net.VpnService;
import android.os.Build;
import android.os.ParcelFileDescriptor;

/**
 * VPN arayuzunu kurar ve bagliyken on planda kalir (kalici bildirim). Arayuzun trafigini Rust
 * tasir: tanitici {@link Native#onTunReady} ile verilir, kapatmak da Rust'in isidir. Uygulamanin
 * kendisi VPN'in disindadir; proxy'nin sitelere actigi baglantilar VPN'e geri girmez.
 */
public class DpiVpnService extends VpnService {
    static final String ACTION_START = "io.github.unsalable.dpiui.START";
    static final String ACTION_STOP = "io.github.unsalable.dpiui.STOP";
    static final String ACTION_BOOT = "io.github.unsalable.dpiui.BOOT";
    static final String EXTRA_TITLE = "title";
    static final String EXTRA_TEXT = "text";
    static final String EXTRA_STOP = "stop";

    private static final String CHANNEL = "status";
    private static final int NOTIFICATION = 1;
    /** VPN icindeki adresler; DNS sorgulari sanal DNS'e (Rust) gider. */
    private static final String ADDRESS_V4 = "10.111.222.1";
    private static final String DNS_V4 = "10.111.222.2";
    private static final String ADDRESS_V6 = "fd00:6470:6975::1";
    private static final int MTU = 1500;

    /** Calisan hizmet (yoksa null); yalnizca ana is parcacigindan okunur. */
    static DpiVpnService instance;

    private String title;
    private String text = "DPI UI";
    private String stopLabel;

    @Override
    public void onCreate() {
        super.onCreate();
        Native.ensureInit(this);
        instance = this;
        title = getString(R.string.protected_title);
        stopLabel = getString(R.string.disconnect);
    }

    @Override
    public void onDestroy() {
        if (instance == this) {
            instance = null;
        }
        super.onDestroy();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        if (ACTION_STOP.equals(action)) {
            // Bildirimdeki "Baglantiyi kes": motor kesince hizmeti de durdurur.
            Native.disconnect();
            return START_NOT_STICKY;
        }
        if (intent != null && intent.hasExtra(EXTRA_TITLE)) {
            title = intent.getStringExtra(EXTRA_TITLE);
            text = intent.getStringExtra(EXTRA_TEXT);
            stopLabel = intent.getStringExtra(EXTRA_STOP);
        }
        goForeground();
        if (ACTION_START.equals(action)) {
            establish();
        } else {
            // Sistem baslatti (her zaman acik VPN) ya da telefon acildi: baglanmayi motor yapar;
            // arayuzu bu hizmete ACTION_START ile yeniden ister.
            Native.connect(ACTION_BOOT.equals(action));
        }
        return START_NOT_STICKY;
    }

    private void establish() {
        Builder builder = new Builder()
                .setSession(getString(R.string.app_name))
                .setMtu(MTU)
                .addAddress(ADDRESS_V4, 30)
                .addRoute("0.0.0.0", 0)
                .addAddress(ADDRESS_V6, 126)
                .addRoute("::", 0)
                .addDnsServer(DNS_V4)
                .setConfigureIntent(openApp());
        try {
            builder.addDisallowedApplication(getPackageName());
        } catch (PackageManager.NameNotFoundException ignored) {
            // Kendi paketimiz her zaman var.
        }
        if (Build.VERSION.SDK_INT >= 29) {
            // Altta mobil veri varsa VPN de "olculu" sayilir.
            builder.setMetered(false);
        }
        ParcelFileDescriptor tun;
        try {
            tun = builder.establish();
        } catch (RuntimeException e) {
            Native.onTunFailed(String.valueOf(e.getMessage()));
            shutdown();
            return;
        }
        if (tun == null) {
            Native.onTunFailed(getString(R.string.no_vpn_permission));
            shutdown();
            return;
        }
        Native.onTunReady(tun.detachFd());
    }

    /** Sistem VPN'i kapatti (ayarlardan ya da baska bir VPN uygulamasi acildi). */
    @Override
    public void onRevoke() {
        Native.onVpnRevoked();
        shutdown();
    }

    void shutdown() {
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    void showStatus(String newTitle, String newText) {
        title = newTitle;
        text = newText;
        refreshNotification();
    }

    void refreshNotification() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null) {
            nm.notify(NOTIFICATION, notification());
        }
    }

    private void goForeground() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null) {
            NotificationChannel channel = new NotificationChannel(CHANNEL, getString(R.string.channel), NotificationManager.IMPORTANCE_LOW);
            channel.setShowBadge(false);
            nm.createNotificationChannel(channel);
        }
        Notification n = notification();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED);
        } else {
            startForeground(NOTIFICATION, n);
        }
    }

    private PendingIntent openApp() {
        Intent open = new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        return PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE);
    }

    private Notification notification() {
        Intent stop = new Intent(this, DpiVpnService.class).setAction(ACTION_STOP);
        PendingIntent stopIntent = PendingIntent.getService(this, 1, stop, PendingIntent.FLAG_IMMUTABLE);
        Notification.Action action = new Notification.Action.Builder(null, stopLabel, stopIntent).build();
        Notification.Builder builder = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat)
                .setContentTitle(title)
                .setContentText(text)
                .setContentIntent(openApp())
                .addAction(action)
                .setOngoing(true)
                .setShowWhen(false)
                .setCategory(Notification.CATEGORY_SERVICE);
        if (Build.VERSION.SDK_INT >= 31) {
            // Bildirim baglanir baglanmaz gorunsun (Android 12+ on plan bildirimini 10 sn geciktirir).
            builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        }
        return builder.build();
    }
}
