"""Verify actual APK pixels, adaptive XML, native UI bytes, identity and ABI."""
import hashlib, io, json, os, re, subprocess, sys, zipfile
from pathlib import Path
from PIL import Image
from approved_branding import ASSETS, check_assets
cfg=check_assets(); builder=Path(sys.argv[1]); out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
apks=list((builder/'output').glob('*.apk'))
if len(apks)!=1: raise SystemExit('Expected one rebuilt APK')
apk=apks[0]
sdk=Path(os.environ['ANDROID_HOME'])/'build-tools';bt=sorted(sdk.iterdir(),key=lambda p:tuple(int(x) for x in p.name.split('.') if x.isdigit()))[-1]
def run(*args): return subprocess.check_output([str(x) for x in args],text=True)
badging=run(bt/'aapt2','dump','badging',apk);resources=run(bt/'aapt2','dump','resources',apk)
cert=run(bt/'apksigner','verify','--verbose','--print-certs',apk)
m=re.search(r"package: name='([^']+)' versionCode='(\d+)' versionName='([^']+)'",badging)
assert m and m.groups()==(cfg['package'],str(cfg['version_code']),cfg['version_name']),m.groups() if m else badging
assert 'launchable-activity:' in badging
assert cfg['version_code']>cfg['previous_version_code']
for name in ('mipmap/ic_launcher','mipmap/ic_launcher_round','drawable/ic_launcher_foreground','color/approved_launcher_background'):
 assert name in resources, name

def pixels(b):
 im=Image.open(io.BytesIO(b)).convert('RGBA');return (im.size,hashlib.sha256(im.tobytes()).hexdigest())
with zipfile.ZipFile(apk) as z:
 names=z.namelist();libs=[n for n in names if n.startswith('lib/') and n.endswith('.so')]
 assert libs and all(n.startswith('lib/arm64-v8a/') for n in libs)
 assert 'classes.dex' in names
 assert not any(n.lower().endswith(('.jks','.p12','.keystore')) for n in names)
 actual={pixels(z.read(n)) for n in names if n.startswith('res/') and n.endswith(('.png','.webp'))}
 for p in (ASSETS/'res').rglob('*.png'):
  assert pixels(p.read_bytes()) in actual,'APK is missing approved pixels: '+str(p)
 rgba=Image.open(ASSETS/'res/drawable-nodpi/ic_inapp_brand_128.png').convert('RGBA').tobytes()
 assert any(rgba in z.read(n) for n in libs),'Native UI brand not embedded'
 xmls=[]
 for n in names:
  if n.startswith('res/') and n.endswith('.xml'):
   dump=run(bt/'aapt2','dump','xmltree',apk,'--file',n)
   if 'E: adaptive-icon' in dump:
    assert 'E: foreground' in dump and 'E: background' in dump
    xmls.append({'path':n,'tree':dump})
 assert len(xmls)>=2,'Missing compiled regular/round adaptive icons'
sha=hashlib.sha256(apk.read_bytes()).hexdigest()
name=cfg['display_name']+'-v'+cfg['version_name']+'-arm64-v8a-EPHEMERAL-NOT-UPDATE.apk'
import shutil
shutil.copyfile(apk,out/name)
(out/'SHA256SUMS.txt').write_text(sha+'  '+name+'\n')
(out/'badging.txt').write_text(badging);(out/'resources.txt').write_text(resources);(out/'certificate.txt').write_text(cert)
(out/'verification.json').write_text(json.dumps({'app':cfg['app'],'version':cfg['version_name'],'versionCode':cfg['version_code'],'apk_sha256':sha,'approved_source_sha256':cfg['source_sha256'],'compiled_pixels_verified':True,'native_ui_brand_embedded':True,'adaptive_xml':xmls,'production_signing_verified':False,'device_tested':False,'eligible_for_release':False},indent=2)+'\n')
print('PASS: compiled APK approved pixels, native UI asset, adaptive XML, package/version and ARM64; production signing and tablet tests still required')
