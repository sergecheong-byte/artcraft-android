import json,subprocess,tempfile,unittest,xml.etree.ElementTree as ET
from pathlib import Path
from PIL import Image
from approved_branding import ASSETS,check_assets
class BrandingTests(unittest.TestCase):
 def test_approved_source_and_safe_area(self):check_assets()
 def test_adaptive_references(self):
  for name in ['ic_launcher','ic_launcher_round']:
   root=ET.parse(ASSETS/f'res/mipmap-anydpi-v26/{name}.xml').getroot()
   self.assertEqual(root.tag,'adaptive-icon');self.assertIsNotNone(root.find('foreground'));self.assertIsNotNone(root.find('background'))
 def test_legacy_sizes(self):
  for density,size in [('mdpi',48),('hdpi',72),('xhdpi',96),('xxhdpi',144),('xxxhdpi',192)]:
   for name in ['ic_launcher','ic_launcher_round']:
    with Image.open(ASSETS/f'res/mipmap-{density}/{name}.png') as im: self.assertEqual(im.size,(size,size))
 def test_monotonic_version(self):
  c=json.loads((ASSETS/'manifest.json').read_text());self.assertGreater(c['version_code'],c['previous_version_code']);self.assertRegex(c['version_name'],r'^\d+\.\d+\.\d+-android\.\d+$')
if __name__=='__main__':unittest.main()
