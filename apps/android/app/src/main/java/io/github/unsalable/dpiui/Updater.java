package io.github.unsalable.dpiui;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.os.Build;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Guncelleme kurulumu: imzasi Rust'ta dogrulanmis APK sistemin paket yukleyicisine verilir.
 * Android 12+ uygulamanin daha once kendi kurdugu surumu onay sormadan gunceller; onay gerekirse
 * kullanici "Simdi yukle" dediyse pencere hemen acilir, kendiliginden kurulumda bildirim gelir.
 */
public class Updater extends BroadcastReceiver {
    private static final String ACTION_RESULT = "io.github.unsalable.dpiui.INSTALL_RESULT";
    private static final String EXTRA_INTERACTIVE = "interactive";
    private static final String CHANNEL = "updates";
    private static final int NOTIFICATION = 2;

    /** Hata metni ya da basariliysa null. Rust'in is parcaciginda calisir. */
    static String install(Context app, String path, boolean interactive) {
        File apk = new File(path);
        try {
            PackageInstaller installer = app.getPackageManager().getPackageInstaller();
            PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
            params.setAppPackageName(app.getPackageName());
            if (Build.VERSION.SDK_INT >= 31) {
                params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED);
            }
            int id = installer.createSession(params);
            try (PackageInstaller.Session session = installer.openSession(id)) {
                try (InputStream in = new FileInputStream(apk);
                        OutputStream out = session.openWrite("base.apk", 0, apk.length())) {
                    byte[] buf = new byte[64 * 1024];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                    }
                    session.fsync(out);
                }
                Intent result = new Intent(app, Updater.class).setAction(ACTION_RESULT).putExtra(EXTRA_INTERACTIVE, interactive);
                // Paket yukleyicisi sonucu bu niyete ekler; degistirilebilir olmali.
                PendingIntent callback = PendingIntent.getBroadcast(app, id, result,
                        PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_MUTABLE);
                session.commit(callback.getIntentSender());
            }
            return null;
        } catch (Exception e) {
            return String.valueOf(e.getMessage());
        }
    }

    @Override
    public void onReceive(Context context, Intent intent) {
        if (!ACTION_RESULT.equals(intent.getAction())) {
            return;
        }
        Native.ensureInit(context);
        int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            Intent confirm = Build.VERSION.SDK_INT >= 33
                    ? intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent.class)
                    : intent.getParcelableExtra(Intent.EXTRA_INTENT);
            if (confirm == null) {
                Native.onInstallResult(false, "onay penceresi yok");
                return;
            }
            confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            if (intent.getBooleanExtra(EXTRA_INTERACTIVE, false) && MainActivity.current != null) {
                MainActivity.current.startActivity(confirm);
            } else {
                notifyReady(context, confirm);
                // Uygulamada "Simdi yukle" yeniden gorunsun (kurulum onay bekliyor).
                Native.onInstallResult(false, "onay bekleniyor");
            }
            return;
        }
        if (status != PackageInstaller.STATUS_SUCCESS) {
            // Kullanici vazgecti ya da kurulum basarisiz: "Simdi yukle" yeniden denenebilir.
            Native.onInstallResult(false, String.valueOf(intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)));
        }
        // Basarida Android uygulamayi kapatip yeni surumu kurar.
    }

    /** Arka planda onay penceresi acilamaz; dokununca acan bir bildirim. */
    private static void notifyReady(Context context, Intent confirm) {
        NotificationManager nm = context.getSystemService(NotificationManager.class);
        if (nm == null) {
            return;
        }
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, context.getString(R.string.updates_channel), NotificationManager.IMPORTANCE_DEFAULT));
        PendingIntent open = PendingIntent.getActivity(context, 0, confirm, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification n = new Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat)
                .setContentTitle(context.getString(R.string.update_ready))
                .setContentText(context.getString(R.string.update_tap))
                .setContentIntent(open)
                .setAutoCancel(true)
                .build();
        nm.notify(NOTIFICATION, n);
    }
}
