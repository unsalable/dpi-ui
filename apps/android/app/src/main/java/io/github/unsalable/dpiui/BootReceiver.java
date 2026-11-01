package io.github.unsalable.dpiui;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.net.VpnService;

import java.io.File;

/**
 * "Telefon acilinca baslat": telefon acilinca (ve uygulama guncellenince) VPN hizmetini baslatir;
 * motor "Acilista otomatik baglan" aciksa baglanir. VPN izni hic verilmediyse bir sey yapmaz.
 */
public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        String action = intent.getAction();
        if (!Intent.ACTION_BOOT_COMPLETED.equals(action) && !Intent.ACTION_MY_PACKAGE_REPLACED.equals(action)) {
            return;
        }
        // Ayar arayuzde secilir; Rust bu dosyayi yazar/siler (androidsys.rs).
        if (!new File(context.getFilesDir(), "autostart").exists() || VpnService.prepare(context) != null) {
            return;
        }
        Intent start = new Intent(context, DpiVpnService.class).setAction(DpiVpnService.ACTION_BOOT);
        context.startForegroundService(start);
    }
}
