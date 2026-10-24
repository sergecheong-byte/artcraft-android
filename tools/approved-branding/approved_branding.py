"""Install owner-approved branding after app-specific Android generation.
No network, signing, package changes, or publication. Fail on source drift.
"""
import argparse, hashlib, json, re, shutil, subprocess, sys
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
ASSETS = HERE / 'assets'

def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('Branding anchor changed: ' + old[:100])
    return text.replace(old, new, 1)

def check_assets():
    cfg = json.loads((ASSETS / 'manifest.json').read_text())
    master = ASSETS / cfg['filename']
    if hashlib.sha256(master.read_bytes()).hexdigest() != cfg['source_sha256']:
        raise ValueError('Approved master checksum mismatch')
    for path, digest in cfg['resources'].items():
        if hashlib.sha256((ASSETS / path).read_bytes()).hexdigest() != digest:
            raise ValueError('Approved derivative checksum mismatch: ' + path)
    fg = Image.open(ASSETS / 'res/drawable-xxxhdpi/ic_launcher_foreground.png').convert('RGBA')
    # Android's 66/108 safe circle, at xxxhdpi (432 pixel canvas).
    if fg.size != (432,432): raise ValueError('Wrong adaptive canvas')
    for y in range(432):
        for x in range(432):
            if fg.getpixel((x,y))[3] > 16 and (x-215.5)**2+(y-215.5)**2 > 132**2:
                raise ValueError('Adaptive artwork leaves guaranteed safe circle')
    return cfg

def ui_brand(source, app):
    root = source / 'crates/ui-egui/src'
    png = ASSETS / 'res/drawable-nodpi/ic_inapp_brand_128.png'
    rgba = Image.open(png).convert('RGBA')
    (root / 'android_approved_brand.rgba').write_bytes(rgba.tobytes())
    (root / 'android_approved_brand.rs').write_text('''// Owner-approved Android branding; one cached texture, no runtime I/O.
pub fn paint(p: &egui::Painter, rect: egui::Rect) {
    let ctx = p.ctx();
    let id = egui::Id::new("android-approved-app-brand");
    let texture = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], include_bytes!("android_approved_brand.rgba"));
        let texture = ctx.load_texture("android-approved-app-brand", image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    let side = rect.width().min(rect.height());
    let rect = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    p.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
}
pub fn show(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect);
    response
}
''')
    lib = root / 'lib.rs'
    s=lib.read_text()
    if 'mod android_approved_brand;' not in s:
        lib.write_text(s+'\n#[cfg(target_os = "android")]\nmod android_approved_brand;\n')
    # Scope changes to existing branding functions, or the native title row where
    # upstream has no app mark. Never replace generic tool icons or file contents.
    hooks = {
      'photocraft':('brand.rs','pub fn paint_mark(ui: &Ui, r: Rect) {','crate::android_approved_brand::paint(ui.painter(), r); return;'),
      'vectorcraft':('brand.rs','pub fn paint_mark(ui: &Ui, r: Rect) {','crate::android_approved_brand::paint(ui.painter(), r); return;'),
      'designcraft':('about.rs','pub fn paint_mark(ui: &egui::Ui, rect: Rect, color: Color32) {','crate::android_approved_brand::paint(ui.painter(), rect); return;'),
      'effectcraft':('header.rs','pub fn paint_logo(p: &egui::Painter, r: Rect) {','crate::android_approved_brand::paint(p, r); return;'),
      'deckcraft':('dialogs.rs','pub fn paint_logo(p: &egui::Painter, r: Rect) {','crate::android_approved_brand::paint(p, r); return;'),
    }
    touched=['lib.rs']
    if app in hooks:
        file,anchor,body=hooks[app];p=root/file
        p.write_text(once(p.read_text(),anchor,anchor+'\n    #[cfg(target_os = "android")] { '+body+' }'))
        touched.append(file)
    elif app=='pdfcraft':
        p=root/'widgets.rs';s=p.read_text()
        for signature,size in [('pub fn artcraft_logo(ui: &mut egui::Ui, height: f32) -> Response {','height'),('pub fn artcraft_mark(ui: &mut egui::Ui, size: f32) -> Response {','size')]:
            s=once(s,signature,signature+'\n    #[cfg(target_os = "android")] { return crate::android_approved_brand::show(ui, '+size+'); }')
        p.write_text(s);touched.append('widgets.rs')
    elif app=='filmcraft':
        p=root/'brand.rs';s=p.read_text();signature='pub fn paint_wordmark(ui: &egui::Ui, left_center: egui::Pos2, height: f32, dark: bool) -> Option<egui::Rect> {'
        body='\n    #[cfg(target_os = "android")] { let r = egui::Rect::from_min_size(egui::pos2(left_center.x, left_center.y-height/2.0), egui::vec2(height,height)); crate::android_approved_brand::paint(ui.painter(),r); return Some(r); }'
        p.write_text(once(s,signature,signature+body));touched.append('brand.rs')
    else:
        file,anchor={
          'lightcraft':('panels/topbar.rs','                ui.spacing_mut().item_spacing.x = 4.0;'),
          'wordcraft':('chrome.rs','                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);'),
          'gridcraft':('ribbon.rs','            ui.add_space(TITLE_LEFT_PAD);'),
        }[app]
        p=root/file;s=p.read_text()
        p.write_text(once(s,anchor,anchor+'\n                #[cfg(target_os = "android")] { crate::android_approved_brand::show(ui, 24.0); }'))
        touched.append(file)
    # Replace embedded app-specific raster icons, including native window/splash
    # images, retaining original dimensions. No document/tool icons are touched.
    for p in (source/'assets/app-icon').rglob('*.png'):
        if app in p.name.lower():
            size=Image.open(p).size
            rgba.resize(size,Image.Resampling.LANCZOS).save(p)
    return touched

def apply(builder, source, android, app):
    cfg=check_assets()
    if cfg['app'] != app: raise ValueError('Wrong application asset mapping')
    res=android/'res'
    # Delete only conflicting launcher definitions (not entire resource folders).
    for p in res.rglob('*'):
        if p.is_file() and p.stem in {'ic_launcher','ic_launcher_round','ic_launcher_foreground'}:
            p.unlink()
    for p in (ASSETS/'res').rglob('*'):
        if not p.is_file(): continue
        relative=p.relative_to(ASSETS/'res')
        # Do not clobber an application's unrelated colors.xml definitions.
        if str(relative)=='values/colors.xml': relative=Path('values/approved_brand_colors.xml')
        dest=res/relative;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,dest)
    # The handoff XML incorrectly concatenated decimal RGB channels (e.g.
    # 6,0,132 -> #0600132). Use its authoritative manifest hex color instead.
    color_file=res/'values/approved_brand_colors.xml'
    color_file.write_text('<?xml version="1.0" encoding="utf-8"?>\n<resources><color name="approved_launcher_background">'+cfg['bg_color']+'</color></resources>\n')
    # Rename the approved background to avoid collision with existing definitions.
    for p in [res/'values/approved_brand_colors.xml',*list((res/'mipmap-anydpi-v26').glob('*.xml'))]:
        p.write_text(p.read_text().replace('artcraft_icon_background','approved_launcher_background').replace('ic_launcher_background','approved_launcher_background'))
    manifest=android/'Cargo.toml';s=manifest.read_text()
    s,n=re.subn(r'^icon\s*=\s*"[^"]+"','icon = "@mipmap/ic_launcher"',s,flags=re.M)
    if n!=1: raise ValueError('Expected exactly one manifest icon')
    s=re.sub(r'^version(?:\.workspace)?\s*=.*$', 'version = "'+cfg['version_name']+'"', s, count=1, flags=re.M)
    s=re.sub(r'^version_code\s*=.*\n', '', s, flags=re.M)
    if not (builder/'lib/android_project.py').exists():
        s=once(s,'[package.metadata.android]','[package.metadata.android]\nversion_code = '+str(cfg['version_code']))
    manifest.write_text(s)
    touched=ui_brand(source,app)
    (android/'approved-branding-evidence.json').write_text(json.dumps({'app':app,'source_sha256':cfg['source_sha256'],'ui_files':touched,'compiled_apk_verified':False,'device_tested':False},indent=2)+'\n')
    print('PASS: approved launcher resources and Android native UI branding installed for '+app)

def install(builder, app):
    cfg=check_assets()
    if cfg['app'] != app: raise ValueError('Wrong application')
    dest=builder/'tools/approved-branding'
    if dest.resolve()!=HERE: shutil.copytree(HERE,dest,dirs_exist_ok=True)
    script=builder/'lib/build-common.sh';s=script.read_text()
    anchor='say 5 "Preparing a builder-owned signing key..."'
    hook='python3 "$ROOT/tools/approved-branding/approved_branding.py" apply "$ROOT" "$ROOT/source/'+app+'" "$ANDROID_DIR" '+app+'\n\n'
    s=once(s,anchor,hook+anchor)
    script.write_text(s)
    project=builder/'lib/android_project.py'
    if project.exists():
        ps=project.read_text()
        anchor='    (android_dir / "AndroidManifest.xml").write_text(manifest_xml(android_meta))'
        ps=once(ps,anchor,'    version = '+repr(cfg['version_name'])+'\n    code = '+str(cfg['version_code'])+'\n'+anchor)
        project.write_text(ps)
    subprocess.run(['bash','-n',str(script)],check=True)
    print('PASS: final branding hook installed after app feature/resource generation')

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('command',choices=['check','install','apply']);p.add_argument('args',nargs='*');a=p.parse_args()
    if a.command=='check': check_assets();print('PASS: approved asset hashes and adaptive safe circle')
    elif a.command=='install': install(Path(a.args[0]).resolve(),a.args[1])
    else: apply(Path(a.args[0]),Path(a.args[1]),Path(a.args[2]),a.args[3])
