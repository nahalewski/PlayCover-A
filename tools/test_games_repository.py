import unittest
from build_games_repository import classify,version_priority

class GamesRepositoryTest(unittest.TestCase):
    def test_emulator_is_excluded_even_when_source_calls_it_game(self):
        for name in ('Delta','PPSSPP','DolphiniOS JIT','emuThreeDS','MAME4iOS','Play!'):
            self.assertEqual(classify({'name':name,'category':'Games'}),'emulators')
    def test_game_utilities_and_uncertain_apps_are_excluded(self):
        self.assertEqual(classify({'name':'Angel Aura Amethyst','category':'Games','localizedDescription':'Minecraft: Java Edition launcher for iOS'}),'other')
        self.assertEqual(classify({'name':'iCube','category':'Games','localizedDescription':'Built on the proven Dolphin emulator foundation'}),'emulators')
        self.assertEqual(classify({'name':'Diamond Finder for Minecraft','category':'Games'}),'other')
        self.assertEqual(classify({'name':'Game news','description':'Read game and emulator reviews'}),'other')
        self.assertEqual(classify({'name':'Unknown','category':'Travel'}),'other')
        self.assertEqual(classify({'name':'Terraria_4.5.0'}),'games')
    def test_numeric_versions_do_not_sort_10_before_9(self):
        records=[({'version':'9'},0,0,''),({'version':'10'},1,0,'')]
        self.assertEqual(sorted(records,key=version_priority,reverse=True)[0][0]['version'],'10')
    def test_published_date_has_priority_over_version_guess(self):
        records=[({'version':'99','date':'2025-01-01'},0,0,''),({'version':'2','date':'2026-01-01'},1,0,'')]
        self.assertEqual(sorted(records,key=version_priority,reverse=True)[0][0]['version'],'2')

if __name__=='__main__':unittest.main()
