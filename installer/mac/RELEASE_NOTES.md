## DPI UI — Mac

Apple Silicon (M1–M4) ve Intel Mac'lerde çalışır; macOS 11 ve sonrası.

### Kurulum
1. Aşağıdan **DPI-UI-Mac.dmg** dosyasını indir ve aç.
2. **DPI UI** simgesini **Uygulamalar** klasörüne sürükle.
3. Uygulamalar'dan DPI UI'yi aç. İlk açılışta macOS "doğrulanamadı" der (uygulama Apple'a ücretli geliştirici hesabıyla kayıtlı değil):
   - **Tamam**'a bas.
   - **Sistem Ayarları → Gizlilik ve Güvenlik**'i aç, en altta "DPI UI engellendi" satırındaki **Yine de Aç**'a bas, şifreni ya da Touch ID'ni gir.
   - Bu yalnızca bir kez gerekir.

   Terminal'i tercih edersen aynı işi şu komut yapar: `xattr -dr com.apple.quarantine "/Applications/DPI UI.app"`

### Kullanım
- Uygulama açılınca kendiliğinden bağlanır; sağ üstte menü çubuğunda dünya simgesi görünür. Bağlıyken simge koyu, kapalıyken soluktur.
- Bir site açılmazsa: **Ayarlar → Bağlantı yöntemi → En iyi yöntemi bul**. Uygulama engelli siteleri her yöntemle dener ve açanı seçer.
- Pencereyi kapatınca uygulama menü çubuğunda çalışmaya devam eder. Tamamen kapatmak için menü çubuğundaki simgeden **DPI UI'den çık**.

### Nasıl çalışır
Mac'te Windows sürümündeki paket sürücüsünün (WinDivert) karşılığı yok. Bu yüzden uygulama bilgisayarda yerel bir proxy açar ve sistem proxy ayarını yalnızca engelli siteler için ona yönlendirir. Diğer siteler doğrudan gider. Bağlantıyı kesince ya da uygulamadan çıkınca ayar eski hâline döner. Yönetici şifresi gerekmez.

Safari, Chrome, Firefox ve Discord masaüstü uygulaması sistem proxy ayarını kullanır. Sistem ayarına uymayan bazı uygulamalar (bazı oyunlar) etkilenmez. Discord sesli sohbet bu sürümde desteklenmiyor olabilir.

### Sorun olursa
**Ayarlar → Genel → Günlüğü aç** ile açılan dosyayı geliştiriciye gönder.
