#!/usr/bin/env python3
"""
HoYo.Gacha 手动插入抽卡记录工具

功能：
- 支持原神、星穹铁道、绝区零三个游戏
- 用户输入5星角色名和抽数，自动生成填充记录
- 直接写入 HoYo.Gacha.v1.db 数据库

使用方法：
1. 将此脚本放在 HoYo.Gacha.v1.db 同目录下
2. 运行: python3 manual_insert_gacha.py
"""

import sqlite3
import json
import os
import sys
from datetime import datetime, timedelta
from typing import Optional, Dict, List, Tuple, Any

# ============================================================================
# 游戏配置
# ============================================================================

GAME_CONFIG = {
    0: {  # 原神
        'name': '原神',
        'gacha_types': {
            100: '新手祈愿',
            200: '常驻祈愿',
            301: '角色祈愿-1',
            400: '角色祈愿-2',
            302: '武器祈愿',
            500: '集录祈愿',
        },
        'default_3star': {'name': '以理服人', 'item_id': 12305, 'item_type': '武器'},
        'default_4star': {'name': '西风剑', 'item_id': 11401, 'item_type': '武器'},
        'rank_types': {3: 3, 4: 4, 5: 5},  # 蓝紫金
        'has_gacha_id': False,
    },
    1: {  # 星穹铁道
        'name': '星穹铁道',
        'gacha_types': {
            1: '常驻跃迁',
            2: '新手跃迁',
            11: '角色跃迁-1',
            12: '角色跃迁-2',
            21: '光锥跃迁-1',
            22: '光锥跃迁-2',
        },
        'default_3star': {'name': '琥珀', 'item_id': 20000, 'item_type': '光锥'},
        'default_4star': {'name': '记忆中的模样', 'item_id': 21000, 'item_type': '光锥'},
        'rank_types': {3: 3, 4: 4, 5: 5},  # 蓝紫金
        'has_gacha_id': True,
    },
    2: {  # 绝区零
        'name': '绝区零',
        'gacha_types': {
            1: '独家频段',
            2: '音擎频段',
            3: '常驻频段',
            5: '邦布频段',
            102: '独家频段-2',
            103: '音擎频段-2',
        },
        'default_3star': {'name': '街头巨星', 'item_id': 21002, 'item_type': '音擎'},
        'default_4star': {'name': '街头涂鸦', 'item_id': 22002, 'item_type': '音擎'},
        'rank_types': {2: 2, 3: 3, 4: 4},  # 蓝紫金（绝区零从2星开始）
        'has_gacha_id': True,
    },
}

# ============================================================================
# 5星角色数据（简化版，从元数据提取）
# ============================================================================

# 格式: {游戏ID: {角色名: {item_id, rank_type, item_type}}}
# 数据从 gacha_metadata.json 自动提取
CHARACTERS_DATA = {
    0: {  # 原神 (64个5星角色)
        '神里绫华': {'item_id': 10000002, 'rank_type': 5, 'item_type': '角色'},
        '琴': {'item_id': 10000003, 'rank_type': 5, 'item_type': '角色'},
        '旅行者': {'item_id': 10000005, 'rank_type': 5, 'item_type': '角色'},
        '迪卢克': {'item_id': 10000016, 'rank_type': 5, 'item_type': '角色'},
        '温迪': {'item_id': 10000022, 'rank_type': 5, 'item_type': '角色'},
        '魈': {'item_id': 10000026, 'rank_type': 5, 'item_type': '角色'},
        '可莉': {'item_id': 10000029, 'rank_type': 5, 'item_type': '角色'},
        '钟离': {'item_id': 10000030, 'rank_type': 5, 'item_type': '角色'},
        '达达利亚': {'item_id': 10000033, 'rank_type': 5, 'item_type': '角色'},
        '七七': {'item_id': 10000035, 'rank_type': 5, 'item_type': '角色'},
        '甘雨': {'item_id': 10000037, 'rank_type': 5, 'item_type': '角色'},
        '阿贝多': {'item_id': 10000038, 'rank_type': 5, 'item_type': '角色'},
        '莫娜': {'item_id': 10000041, 'rank_type': 5, 'item_type': '角色'},
        '刻晴': {'item_id': 10000042, 'rank_type': 5, 'item_type': '角色'},
        '胡桃': {'item_id': 10000046, 'rank_type': 5, 'item_type': '角色'},
        '枫原万叶': {'item_id': 10000047, 'rank_type': 5, 'item_type': '角色'},
        '宵宫': {'item_id': 10000049, 'rank_type': 5, 'item_type': '角色'},
        '优菈': {'item_id': 10000051, 'rank_type': 5, 'item_type': '角色'},
        '雷电将军': {'item_id': 10000052, 'rank_type': 5, 'item_type': '角色'},
        '珊瑚宫心海': {'item_id': 10000054, 'rank_type': 5, 'item_type': '角色'},
        '荒泷一斗': {'item_id': 10000057, 'rank_type': 5, 'item_type': '角色'},
        '八重神子': {'item_id': 10000058, 'rank_type': 5, 'item_type': '角色'},
        '夜兰': {'item_id': 10000060, 'rank_type': 5, 'item_type': '角色'},
        '埃洛伊': {'item_id': 10000062, 'rank_type': 5, 'item_type': '角色'},
        '申鹤': {'item_id': 10000063, 'rank_type': 5, 'item_type': '角色'},
        '神里绫人': {'item_id': 10000066, 'rank_type': 5, 'item_type': '角色'},
        '提纳里': {'item_id': 10000069, 'rank_type': 5, 'item_type': '角色'},
        '妮露': {'item_id': 10000070, 'rank_type': 5, 'item_type': '角色'},
        '赛诺': {'item_id': 10000071, 'rank_type': 5, 'item_type': '角色'},
        '纳西妲': {'item_id': 10000073, 'rank_type': 5, 'item_type': '角色'},
        '流浪者': {'item_id': 10000075, 'rank_type': 5, 'item_type': '角色'},
        '艾尔海森': {'item_id': 10000078, 'rank_type': 5, 'item_type': '角色'},
        '迪希雅': {'item_id': 10000079, 'rank_type': 5, 'item_type': '角色'},
        '白术': {'item_id': 10000082, 'rank_type': 5, 'item_type': '角色'},
        '林尼': {'item_id': 10000084, 'rank_type': 5, 'item_type': '角色'},
        '莱欧斯利': {'item_id': 10000086, 'rank_type': 5, 'item_type': '角色'},
        '那维莱特': {'item_id': 10000087, 'rank_type': 5, 'item_type': '角色'},
        '芙宁娜': {'item_id': 10000089, 'rank_type': 5, 'item_type': '角色'},
        '娜维娅': {'item_id': 10000091, 'rank_type': 5, 'item_type': '角色'},
        '闲云': {'item_id': 10000093, 'rank_type': 5, 'item_type': '角色'},
        '千织': {'item_id': 10000094, 'rank_type': 5, 'item_type': '角色'},
        '希格雯': {'item_id': 10000095, 'rank_type': 5, 'item_type': '角色'},
        '阿蕾奇诺': {'item_id': 10000096, 'rank_type': 5, 'item_type': '角色'},
        '克洛琳德': {'item_id': 10000098, 'rank_type': 5, 'item_type': '角色'},
        '艾梅莉埃': {'item_id': 10000099, 'rank_type': 5, 'item_type': '角色'},
        '基尼奇': {'item_id': 10000101, 'rank_type': 5, 'item_type': '角色'},
        '玛拉妮': {'item_id': 10000102, 'rank_type': 5, 'item_type': '角色'},
        '希诺宁': {'item_id': 10000103, 'rank_type': 5, 'item_type': '角色'},
        '恰斯卡': {'item_id': 10000104, 'rank_type': 5, 'item_type': '角色'},
        '玛薇卡': {'item_id': 10000106, 'rank_type': 5, 'item_type': '角色'},
        '茜特菈莉': {'item_id': 10000107, 'rank_type': 5, 'item_type': '角色'},
        '梦见月瑞希': {'item_id': 10000109, 'rank_type': 5, 'item_type': '角色'},
        '瓦雷莎': {'item_id': 10000111, 'rank_type': 5, 'item_type': '角色'},
        '爱可菲': {'item_id': 10000112, 'rank_type': 5, 'item_type': '角色'},
        '丝柯克': {'item_id': 10000114, 'rank_type': 5, 'item_type': '角色'},
        '伊涅芙': {'item_id': 10000116, 'rank_type': 5, 'item_type': '角色'},
        '奇偶·男性': {'item_id': 10000117, 'rank_type': 5, 'item_type': '角色'},
        '奇偶·女性': {'item_id': 10000118, 'rank_type': 5, 'item_type': '角色'},
        '菈乌玛': {'item_id': 10000119, 'rank_type': 5, 'item_type': '角色'},
        '菲林斯': {'item_id': 10000120, 'rank_type': 5, 'item_type': '角色'},
        '奈芙尔': {'item_id': 10000122, 'rank_type': 5, 'item_type': '角色'},
        '杜林': {'item_id': 10000123, 'rank_type': 5, 'item_type': '角色'},
        '哥伦比娅': {'item_id': 10000125, 'rank_type': 5, 'item_type': '角色'},
        '兹白': {'item_id': 10000126, 'rank_type': 5, 'item_type': '角色'},
    },
    1: {  # 星穹铁道 (54个5星角色)
        '姬子': {'item_id': 1003, 'rank_type': 5, 'item_type': '角色'},
        '瓦尔特': {'item_id': 1004, 'rank_type': 5, 'item_type': '角色'},
        '卡芙卡': {'item_id': 1005, 'rank_type': 5, 'item_type': '角色'},
        '银狼': {'item_id': 1006, 'rank_type': 5, 'item_type': '角色'},
        'Saber': {'item_id': 1014, 'rank_type': 5, 'item_type': '角色'},
        'Archer': {'item_id': 1015, 'rank_type': 5, 'item_type': '角色'},
        '布洛妮娅': {'item_id': 1101, 'rank_type': 5, 'item_type': '角色'},
        '希儿': {'item_id': 1102, 'rank_type': 5, 'item_type': '角色'},
        '杰帕德': {'item_id': 1104, 'rank_type': 5, 'item_type': '角色'},
        '克拉拉': {'item_id': 1107, 'rank_type': 5, 'item_type': '角色'},
        '托帕&账账': {'item_id': 1112, 'rank_type': 5, 'item_type': '角色'},
        '罗刹': {'item_id': 1203, 'rank_type': 5, 'item_type': '角色'},
        '景元': {'item_id': 1204, 'rank_type': 5, 'item_type': '角色'},
        '刃': {'item_id': 1205, 'rank_type': 5, 'item_type': '角色'},
        '符玄': {'item_id': 1208, 'rank_type': 5, 'item_type': '角色'},
        '彦卿': {'item_id': 1209, 'rank_type': 5, 'item_type': '角色'},
        '白露': {'item_id': 1211, 'rank_type': 5, 'item_type': '角色'},
        '镜流': {'item_id': 1212, 'rank_type': 5, 'item_type': '角色'},
        '丹恒•饮月': {'item_id': 1213, 'rank_type': 5, 'item_type': '角色'},
        '藿藿': {'item_id': 1217, 'rank_type': 5, 'item_type': '角色'},
        '椒丘': {'item_id': 1218, 'rank_type': 5, 'item_type': '角色'},
        '飞霄': {'item_id': 1220, 'rank_type': 5, 'item_type': '角色'},
        '云璃': {'item_id': 1221, 'rank_type': 5, 'item_type': '角色'},
        '灵砂': {'item_id': 1222, 'rank_type': 5, 'item_type': '角色'},
        '忘归人': {'item_id': 1225, 'rank_type': 5, 'item_type': '角色'},
        '银枝': {'item_id': 1302, 'rank_type': 5, 'item_type': '角色'},
        '阮•梅': {'item_id': 1303, 'rank_type': 5, 'item_type': '角色'},
        '砂金': {'item_id': 1304, 'rank_type': 5, 'item_type': '角色'},
        '真理医生': {'item_id': 1305, 'rank_type': 5, 'item_type': '角色'},
        '花火': {'item_id': 1306, 'rank_type': 5, 'item_type': '角色'},
        '黑天鹅': {'item_id': 1307, 'rank_type': 5, 'item_type': '角色'},
        '黄泉': {'item_id': 1308, 'rank_type': 5, 'item_type': '角色'},
        '知更鸟': {'item_id': 1309, 'rank_type': 5, 'item_type': '角色'},
        '流萤': {'item_id': 1310, 'rank_type': 5, 'item_type': '角色'},
        '星期日': {'item_id': 1313, 'rank_type': 5, 'item_type': '角色'},
        '翡翠': {'item_id': 1314, 'rank_type': 5, 'item_type': '角色'},
        '波提欧': {'item_id': 1315, 'rank_type': 5, 'item_type': '角色'},
        '乱破': {'item_id': 1317, 'rank_type': 5, 'item_type': '角色'},
        '大丽花': {'item_id': 1321, 'rank_type': 5, 'item_type': '角色'},
        '大黑塔': {'item_id': 1401, 'rank_type': 5, 'item_type': '角色'},
        '阿格莱雅': {'item_id': 1402, 'rank_type': 5, 'item_type': '角色'},
        '缇宝': {'item_id': 1403, 'rank_type': 5, 'item_type': '角色'},
        '万敌': {'item_id': 1404, 'rank_type': 5, 'item_type': '角色'},
        '那刻夏': {'item_id': 1405, 'rank_type': 5, 'item_type': '角色'},
        '赛飞儿': {'item_id': 1406, 'rank_type': 5, 'item_type': '角色'},
        '遐蝶': {'item_id': 1407, 'rank_type': 5, 'item_type': '角色'},
        '白厄': {'item_id': 1408, 'rank_type': 5, 'item_type': '角色'},
        '风堇': {'item_id': 1409, 'rank_type': 5, 'item_type': '角色'},
        '海瑟音': {'item_id': 1410, 'rank_type': 5, 'item_type': '角色'},
        '刻律德菈': {'item_id': 1412, 'rank_type': 5, 'item_type': '角色'},
        '长夜月': {'item_id': 1413, 'rank_type': 5, 'item_type': '角色'},
        '丹恒•腾荒': {'item_id': 1414, 'rank_type': 5, 'item_type': '角色'},
        '昔涟': {'item_id': 1415, 'rank_type': 5, 'item_type': '角色'},
    },
    2: {  # 绝区零 (34个S级代理人)
        '猫又': {'item_id': 1021, 'rank_type': 4, 'item_type': '代理人'},
        '「11号」': {'item_id': 1041, 'rank_type': 4, 'item_type': '代理人'},
        '伊德海莉': {'item_id': 1051, 'rank_type': 4, 'item_type': '代理人'},
        '凯撒': {'item_id': 1071, 'rank_type': 4, 'item_type': '代理人'},
        '雅': {'item_id': 1091, 'rank_type': 4, 'item_type': '代理人'},
        '珂蕾妲': {'item_id': 1101, 'rank_type': 4, 'item_type': '代理人'},
        '莱卡恩': {'item_id': 1141, 'rank_type': 4, 'item_type': '代理人'},
        '莱特': {'item_id': 1161, 'rank_type': 4, 'item_type': '代理人'},
        '柏妮思': {'item_id': 1171, 'rank_type': 4, 'item_type': '代理人'},
        '格莉丝': {'item_id': 1181, 'rank_type': 4, 'item_type': '代理人'},
        '艾莲': {'item_id': 1191, 'rank_type': 4, 'item_type': '代理人'},
        '悠真': {'item_id': 1201, 'rank_type': 4, 'item_type': '代理人'},
        '丽娜': {'item_id': 1211, 'rank_type': 4, 'item_type': '代理人'},
        '柳': {'item_id': 1221, 'rank_type': 4, 'item_type': '代理人'},
        '朱鸢': {'item_id': 1241, 'rank_type': 4, 'item_type': '代理人'},
        '青衣': {'item_id': 1251, 'rank_type': 4, 'item_type': '代理人'},
        '简': {'item_id': 1261, 'rank_type': 4, 'item_type': '代理人'},
        '雨果': {'item_id': 1291, 'rank_type': 4, 'item_type': '代理人'},
        '奥菲丝&「鬼火」': {'item_id': 1301, 'rank_type': 4, 'item_type': '代理人'},
        '耀嘉音': {'item_id': 1311, 'rank_type': 4, 'item_type': '代理人'},
        '伊芙琳': {'item_id': 1321, 'rank_type': 4, 'item_type': '代理人'},
        '薇薇安': {'item_id': 1331, 'rank_type': 4, 'item_type': '代理人'},
        '照': {'item_id': 1341, 'rank_type': 4, 'item_type': '代理人'},
        '「扳机」': {'item_id': 1361, 'rank_type': 4, 'item_type': '代理人'},
        '仪玄': {'item_id': 1371, 'rank_type': 4, 'item_type': '代理人'},
        '零号·安比': {'item_id': 1381, 'rank_type': 4, 'item_type': '代理人'},
        '橘福福': {'item_id': 1391, 'rank_type': 4, 'item_type': '代理人'},
        '爱丽丝': {'item_id': 1401, 'rank_type': 4, 'item_type': '代理人'},
        '柚叶': {'item_id': 1411, 'rank_type': 4, 'item_type': '代理人'},
        '叶瞬光': {'item_id': 1431, 'rank_type': 4, 'item_type': '代理人'},
        '卢西娅': {'item_id': 1451, 'rank_type': 4, 'item_type': '代理人'},
        '「席德」': {'item_id': 1461, 'rank_type': 4, 'item_type': '代理人'},
        '般岳': {'item_id': 1471, 'rank_type': 4, 'item_type': '代理人'},
        '琉音': {'item_id': 1481, 'rank_type': 4, 'item_type': '代理人'},
    },
}


def generate_id(timestamp: datetime, sequence: int = 0) -> str:
    """
    根据时间戳生成伪造ID
    格式: 时间戳(10位) + 序列号(9位)
    """
    ts = int(timestamp.timestamp())
    return f"{ts}{sequence:09d}"


def find_db_path() -> Optional[str]:
    """查找数据库文件"""
    # 当前目录
    db_names = ['HoYo.Gacha.v1.db', '__DEV__HoYo.Gacha.v1.db']

    for name in db_names:
        if os.path.exists(name):
            return name

    return None


def get_existing_accounts(conn: sqlite3.Connection) -> List[Tuple[int, int]]:
    """获取已存在的账号列表"""
    cursor = conn.execute(
        "SELECT DISTINCT business, uid FROM HG_ACCOUNTS ORDER BY business, uid"
    )
    return cursor.fetchall()


def get_max_id_for_gacha_type(conn: sqlite3.Connection, business: int, uid: int, gacha_type: int) -> Optional[str]:
    """获取指定卡池的最大ID"""
    cursor = conn.execute(
        "SELECT MAX(id) FROM HG_GACHA_RECORDS WHERE business = ? AND uid = ? AND gacha_type = ?",
        (business, uid, gacha_type)
    )
    result = cursor.fetchone()[0]
    return result


def check_character_exists(business: int, character_name: str) -> Optional[Dict]:
    """检查角色是否存在并返回信息"""
    if business not in CHARACTERS_DATA:
        return None
    return CHARACTERS_DATA[business].get(character_name)


def insert_records(
    conn: sqlite3.Connection,
    business: int,
    uid: int,
    gacha_type: int,
    golden_character: str,
    golden_info: Dict,
    pull_count: int,
    end_time: datetime,
    lang: str = 'zh-cn',
) -> int:
    """
    插入抽卡记录

    逻辑：
    - 第1抽时间最早，第N抽（5星）时间最晚（end_time）
    - 每8抽一个4星（第8、16、24...抽）
    - 最后一次抽是5星

    返回插入的记录数
    """
    config = GAME_CONFIG[business]
    default_3star = config['default_3star']
    default_4star = config['default_4star']
    has_gacha_id = config['has_gacha_id']

    records = []

    # 第1抽的时间 = end_time 往前推 (pull_count - 1) 秒
    first_pull_time = end_time - timedelta(seconds=pull_count - 1)

    for i in range(pull_count):
        pull_number = i + 1  # 第几抽 (1-based)
        current_time = first_pull_time + timedelta(seconds=i)

        # 判断这一抽是什么
        if pull_number == pull_count:
            # 最后一抽是5星
            name = golden_character
            item_id = golden_info['item_id']
            rank_type = golden_info['rank_type']
            item_type = golden_info['item_type']
        elif pull_number % 8 == 0:
            # 每8抽是4星（第8、16、24...）
            name = default_4star['name']
            item_id = default_4star['item_id']
            rank_type = 4
            item_type = default_4star['item_type']
        else:
            # 其他是3星
            name = default_3star['name']
            item_id = default_3star['item_id']
            rank_type = 3
            item_type = default_3star['item_type']

        # 绝区零的rank_type不同
        if business == 2:
            if rank_type == 3:
                rank_type = 2  # 绝区零的蓝是2星
            elif rank_type == 4:
                rank_type = 3  # 绝区零的紫是3星
            elif rank_type == 5:
                rank_type = 4  # 绝区零的金是4星

        # 生成ID（基于时间戳，时间早的ID小）
        ts = int(current_time.timestamp())
        record_id = f"{ts}000000000"

        # 格式化时间
        time_str = current_time.strftime('%Y-%m-%dT%H:%M:%S+08:00')

        records.append({
            'business': business,
            'uid': uid,
            'id': record_id,
            'gacha_type': gacha_type,
            'gacha_id': None if not has_gacha_id else int(current_time.timestamp()),
            'rank_type': rank_type,
            'count': 1,
            'time': time_str,
            'lang': lang,
            'name': name,
            'item_type': item_type,
            'item_id': str(item_id),
        })

    # 插入数据库
    cursor = conn.cursor()
    inserted = 0

    for record in records:
        try:
            cursor.execute("""
                INSERT INTO HG_GACHA_RECORDS (
                    business, uid, id, gacha_type, gacha_id,
                    rank_type, count, time, lang, name, item_type, item_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """, (
                record['business'],
                record['uid'],
                record['id'],
                record['gacha_type'],
                record['gacha_id'],
                record['rank_type'],
                record['count'],
                record['time'],
                record['lang'],
                record['name'],
                record['item_type'],
                record['item_id'],
            ))
            inserted += 1
        except sqlite3.IntegrityError:
            # 主键冲突，跳过
            pass

    conn.commit()
    return inserted


def print_banner():
    print("=" * 60)
    print("  HoYo.Gacha 手动插入抽卡记录工具")
    print("=" * 60)
    print()


def select_game() -> int:
    """选择游戏"""
    print("请选择游戏:")
    for biz_id, config in GAME_CONFIG.items():
        print(f"  [{biz_id}] {config['name']}")

    while True:
        try:
            choice = input("\n输入选项 (0/1/2): ").strip()
            biz = int(choice)
            if biz in GAME_CONFIG:
                return biz
            print("无效选项，请重新输入")
        except ValueError:
            print("请输入数字")


def select_gacha_type(business: int) -> int:
    """选择卡池"""
    config = GAME_CONFIG[business]
    print(f"\n请选择卡池:")
    for gacha_type, name in config['gacha_types'].items():
        print(f"  [{gacha_type}] {name}")

    while True:
        try:
            choice = input("\n输入卡池编号: ").strip()
            gacha_type = int(choice)
            if gacha_type in config['gacha_types']:
                return gacha_type
            print("无效选项，请重新输入")
        except ValueError:
            print("请输入数字")


def input_character(business: int) -> Tuple[str, Dict]:
    """输入角色名"""
    characters = CHARACTERS_DATA.get(business, {})

    print(f"\n可用角色 (共{len(characters)}个5星):")
    # 分列显示
    names = list(characters.keys())
    for i in range(0, len(names), 5):
        print("  " + ", ".join(names[i:i+5]))

    while True:
        name = input("\n输入5星角色名: ").strip()
        if name in characters:
            return name, characters[name]
        print(f"未找到角色: {name}，请重新输入")


def input_datetime() -> datetime:
    """输入时间"""
    print("\n输入抽中时间 (格式: YYYY-MM-DD HH:MM:SS)")
    print("示例: 2023-11-08 21:06:34")

    while True:
        time_str = input("时间: ").strip()
        try:
            return datetime.strptime(time_str, "%Y-%m-%d %H:%M:%S")
        except ValueError:
            print("格式错误，请按 YYYY-MM-DD HH:MM:SS 格式输入")


def main():
    print_banner()

    # 查找数据库
    db_path = find_db_path()
    if not db_path:
        print("错误: 未找到 HoYo.Gacha.v1.db 文件")
        print("请将此脚本放在数据库文件同目录下运行")
        sys.exit(1)

    print(f"找到数据库: {db_path}\n")

    # 连接数据库
    conn = sqlite3.connect(db_path)

    # 显示已有账号
    accounts = get_existing_accounts(conn)
    if accounts:
        print("已有账号:")
        for biz, uid in accounts:
            print(f"  {GAME_CONFIG.get(biz, {}).get('name', biz)}: {uid}")
        print()

    # 选择游戏
    business = select_game()

    # 输入UID
    while True:
        try:
            uid = int(input("\n输入UID: ").strip())
            if uid > 0:
                break
            print("UID必须大于0")
        except ValueError:
            print("请输入数字")

    # 选择卡池
    gacha_type = select_gacha_type(business)

    # 输入角色
    character_name, character_info = input_character(business)

    # 输入时间
    end_time = input_datetime()

    # 输入抽数
    while True:
        try:
            pull_count = int(input("\n输入抽取次数: ").strip())
            if pull_count > 0:
                break
            print("抽数必须大于0")
        except ValueError:
            print("请输入数字")

    # 计算记录分布
    four_star_count = (pull_count - 1) // 8
    three_star_count = pull_count - 1 - four_star_count

    # 确认
    print("\n" + "=" * 60)
    print("确认插入以下记录:")
    print(f"  游戏: {GAME_CONFIG[business]['name']}")
    print(f"  UID: {uid}")
    print(f"  卡池: {GAME_CONFIG[business]['gacha_types'][gacha_type]}")
    print(f"  5星: {character_name}")
    print(f"  抽取时间: {end_time.strftime('%Y-%m-%d %H:%M:%S')}")
    print(f"  总抽数: {pull_count}")
    print(f"  记录分布: 1个5星 + {four_star_count}个4星 + {three_star_count}个3星")
    print("=" * 60)

    confirm = input("\n确认插入? (y/N): ").strip().lower()
    if confirm != 'y':
        print("已取消")
        conn.close()
        sys.exit(0)

    # 插入记录
    print("\n正在插入记录...")
    inserted = insert_records(
        conn=conn,
        business=business,
        uid=uid,
        gacha_type=gacha_type,
        golden_character=character_name,
        golden_info=character_info,
        pull_count=pull_count,
        end_time=end_time,
    )

    print(f"成功插入 {inserted} 条记录")

    conn.close()


if __name__ == '__main__':
    main()