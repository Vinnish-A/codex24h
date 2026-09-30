#!/usr/bin/env python3
import importlib.machinery
import json
import os
import time
import e2e
from pathlib import Path
import tempfile
import unittest

helper = importlib.machinery.SourceFileLoader('requests_helper', str(Path(__file__).resolve().parents[1]/'scripts/codex24h-requests')).load_module()

class RequestsTest(unittest.TestCase):
    def read(self, records, partial=''):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'rollout-test.jsonl'
            path.write_text(''.join(json.dumps(v)+'\n' for v in records)+partial)
            return helper.read_session(path)
    def test_event_mirror_does_not_duplicate_but_repeated_requests_survive(self):
        event={'type':'event_msg','payload':{'type':'user_message','message':'重复请求'}}
        mirror={'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'重复请求'}]}}
        answer={'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'回答'}]}}
        tool={'type':'response_item','payload':{'type':'function_call_output','output':'RAW_TOOL_PAYLOAD'}}
        result=self.read([event,mirror,answer,tool,event,mirror,answer],'{"partial":')
        self.assertEqual([r['text'] for r in result['requests']],['重复请求']*2)
        self.assertNotIn('entries',result)
        self.assertNotIn('回答',json.dumps(result,ensure_ascii=False))
        self.assertNotIn('RAW_TOOL_PAYLOAD',json.dumps(result))
    def test_response_only_schema_images_and_environment(self):
        def user(text):return {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':text}]}}
        picture={'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_image','image_url':'data:not-rendered'}]}}
        result=self.read([user('<environment_context>runtime</environment_context>'),user('first'),picture,user('first')])
        self.assertEqual([r['text'] for r in result['requests']],['first','','first'])
    def test_owned_rollout_does_not_pick_newer_other_session(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp)
            ours=home/'rollout-ours.jsonl';ours.touch()
            (home/'rollout-newer.jsonl').touch()
            with ours.open():
                self.assertEqual(helper.rollout(os.getpid(),home),ours)
    def test_completed_corrupt_record_reports_error(self):
        with self.assertRaises(json.JSONDecodeError):self.read([], 'bad record\n')
if __name__=='__main__':unittest.main()
