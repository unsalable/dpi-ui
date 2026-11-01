package io.github.unsalable.dpiui;

import android.app.PendingIntent;
import android.content.Intent;
import android.net.VpnService;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.service.quicksettings.Tile;
import android.service.quicksettings.TileService;

/** Bildirim panelindeki hizli ayar: masaustundeki tepsi menusunun "Baglan / Kes"i. */
public class ToggleTile extends TileService {
    private final Handler handler = new Handler(Looper.getMainLooper());

    @Override
    public void onStartListening() {
        Native.ensureInit(this);
        refresh();
    }

    @Override
    public void onClick() {
        Native.ensureInit(this);
        if (Native.isConnected()) {
            Native.disconnect();
        } else if (VpnService.prepare(this) != null) {
            // Ilk baglanti: VPN izni uygulamanin icinde istenir.
            openApp();
            return;
        } else {
            Native.connect(false);
        }
        // Baglanma birkac yuz milisaniye surer; durum sonra yenilenir.
        handler.postDelayed(this::refresh, 600);
        handler.postDelayed(this::refresh, 2000);
    }

    private void openApp() {
        Intent open = new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE));
        } else {
            startActivityAndCollapse(open);
        }
    }

    private void refresh() {
        Tile tile = getQsTile();
        if (tile == null) {
            return;
        }
        boolean on = Native.isConnected();
        tile.setState(on ? Tile.STATE_ACTIVE : Tile.STATE_INACTIVE);
        if (Build.VERSION.SDK_INT >= 29) {
            tile.setSubtitle(Native.statusLabel(on));
        }
        tile.updateTile();
    }
}
