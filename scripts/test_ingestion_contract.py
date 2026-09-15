"""Generator/compatibility guard regression tests; never edit the real contract."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('contract',Path(__file__).with_name('ingestion_contract.py'))
contract=importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)

class ContractGuards(unittest.TestCase):
 def test_schema_edit_changes_types_and_manifest(self):
  original=contract.ROOT
  with tempfile.TemporaryDirectory() as directory:
   shutil.copytree(original,Path(directory)/'contract')
   contract.ROOT=Path(directory)/'contract'
   try:
    before=contract.outputs()
    p=contract.ROOT/'batch.schema.json';schema=json.loads(p.read_text())
    schema['properties']['project_id']['type']='integer'
    p.write_text(json.dumps(schema))
    after=contract.outputs()
    self.assertNotEqual(before['types.generated.ts'],after['types.generated.ts'])
    self.assertNotEqual(before['manifest.json'],after['manifest.json'])
    with self.assertRaises(ValueError):contract.require_version_change(json.loads(before['manifest.json']),json.loads(after['manifest.json']))
   finally:contract.ROOT=original
 def test_behavior_change_also_requires_version(self):
  old={'contract_version':'1.0.0','files':{'behavior.md':'old'}}
  new={'contract_version':'1.0.0','files':{'behavior.md':'changed'}}
  with self.assertRaises(ValueError):contract.require_version_change(old,new)
  contract.require_version_change(old,dict(new,contract_version='2.0.0'))
if __name__=='__main__':unittest.main()
