# slug, display name, Horizons COMMAND, primary slug, tags
# Moons use their planet as CENTER so the elements are planetocentric.
SUN = ("sol", "Sol", "10", None, ["Star"])

PLANETS = [
    ("mercury", "Mercury", "199", "sol", ["Planet", "Major Planet"]),
    ("venus",   "Venus",   "299", "sol", ["Planet", "Major Planet"]),
    ("earth",   "Earth",   "399", "sol", ["Planet", "Major Planet"]),
    ("mars",    "Mars",    "499", "sol", ["Planet", "Major Planet"]),
    ("Jupiter", "Jupiter", "599", "sol", ["Planet", "Major Planet"]),
    ("saturn",  "Saturn",  "699", "sol", ["Planet", "Major Planet"]),
    ("Uranus",  "Uranus",  "799", "sol", ["Planet", "Major Planet"]),
    ("Neptune", "Neptune", "899", "sol", ["Planet", "Major Planet"]),
    ("pluto",   "Pluto",   "999", "sol", ["Dwarf Planet", "TNO"]),
]

MOONS = [
    ("luna", "Luna", "301", "earth"),
    ("phobos", "Phobos", "401", "mars"), ("deimos", "Deimos", "402", "mars"),
    # Jovian
    ("io", "Io", "501", "Jupiter"), ("europa", "Europa", "502", "Jupiter"),
    ("ganymede", "Ganymede", "503", "Jupiter"), ("callisto", "Callisto", "504", "Jupiter"),
    ("amalthea", "Amalthea", "505", "Jupiter"), ("himalia", "Himalia", "506", "Jupiter"),
    ("elara", "Elara", "507", "Jupiter"), ("pasiphae", "Pasiphae", "508", "Jupiter"),
    ("sinope", "Sinope", "509", "Jupiter"), ("lysithea", "Lysithea", "510", "Jupiter"),
    ("carme", "Carme", "511", "Jupiter"), ("ananke", "Ananke", "512", "Jupiter"),
    ("leda", "Leda", "513", "Jupiter"), ("thebe", "Thebe", "514", "Jupiter"),
    ("adrastea", "Adrastea", "515", "Jupiter"), ("metis", "Metis", "516", "Jupiter"),
    # Saturnian
    ("mimas", "Mimas", "601", "saturn"), ("enceladus", "Enceladus", "602", "saturn"),
    ("tethys", "Tethys", "603", "saturn"), ("dione", "Dione", "604", "saturn"),
    ("rhea", "Rhea", "605", "saturn"), ("titan", "Titan", "606", "saturn"),
    ("hyperion", "Hyperion", "607", "saturn"), ("iapetus", "Iapetus", "608", "saturn"),
    ("phoebe", "Phoebe", "609", "saturn"), ("janus", "Janus", "610", "saturn"),
    ("epimetheus", "Epimetheus", "611", "saturn"), ("helene", "Helene", "612", "saturn"),
    ("atlas", "Atlas", "615", "saturn"), ("prometheus", "Prometheus", "616", "saturn"),
    ("pandora", "Pandora", "617", "saturn"), ("pan", "Pan", "618", "saturn"),
    # Uranian
    ("ariel", "Ariel", "701", "Uranus"), ("umbriel", "Umbriel", "702", "Uranus"),
    ("titania", "Titania", "703", "Uranus"), ("oberon", "Oberon", "704", "Uranus"),
    ("miranda", "Miranda", "705", "Uranus"), ("puck", "Puck", "715", "Uranus"),
    # Neptunian
    ("triton", "Triton", "801", "Neptune"), ("nereid", "Nereid", "802", "Neptune"),
    ("despina", "Despina", "805", "Neptune"), ("larissa", "Larissa", "807", "Neptune"),
    ("proteus", "Proteus", "808", "Neptune"),
    # Plutonian
    ("charon", "Charon", "901", "pluto"), ("nix", "Nix", "902", "pluto"),
    ("hydra", "Hydra", "903", "pluto"), ("kerberos", "Kerberos", "904", "pluto"),
    ("styx", "Styx", "905", "pluto"),
]

ASTEROIDS = [
    ("1-ceres", "Ceres", "1;", ["Dwarf Planet", "Asteroid"]),
    ("2-pallas", "Pallas", "2;", ["Asteroid"]),
    ("3-juno", "Juno", "3;", ["Asteroid"]),
    ("4-vesta", "Vesta", "4;", ["Asteroid"]),
    ("10-hygiea", "Hygiea", "10;", ["Asteroid"]),
    ("15-eunomia", "Eunomia", "15;", ["Asteroid"]),
    ("16-psyche", "Psyche", "16;", ["Asteroid"]),
    ("21-lutetia", "Lutetia", "21;", ["Asteroid"]),
    ("52-europa", "52 Europa", "52;", ["Asteroid"]),
    ("87-sylvia", "Sylvia", "87;", ["Asteroid"]),
    ("216-kleopatra", "Kleopatra", "216;", ["Asteroid"]),
    ("243-ida", "Ida", "243;", ["Asteroid"]),
    ("253-mathilde", "Mathilde", "253;", ["Asteroid"]),
    ("433-eros", "Eros", "433;", ["Asteroid", "NEO"]),
    ("511-davida", "Davida", "511;", ["Asteroid"]),
    ("704-interamnia", "Interamnia", "704;", ["Asteroid"]),
    ("951-gaspra", "Gaspra", "951;", ["Asteroid"]),
    ("1566-icarus", "Icarus", "1566;", ["Asteroid", "NEO"]),
    ("1620-geographos", "Geographos", "1620;", ["Asteroid", "NEO"]),
    ("2867-steins", "Steins", "2867;", ["Asteroid"]),
    ("3200-phaethon", "Phaethon", "3200;", ["Asteroid", "NEO"]),
    ("4179-toutatis", "Toutatis", "4179;", ["Asteroid", "NEO"]),
    ("25143-itokawa", "Itokawa", "25143;", ["Asteroid", "NEO"]),
    ("99942-apophis", "Apophis", "99942;", ["Asteroid", "NEO"]),
    ("101955-bennu", "Bennu", "101955;", ["Asteroid", "NEO"]),
    ("162173-ryugu", "Ryugu", "162173;", ["Asteroid", "NEO"]),
]

TNOS = [
    ("eris", "Eris", "136199;", ["Dwarf Planet", "TNO"]),
    ("136472-makemake", "Makemake", "136472;", ["Dwarf Planet", "TNO"]),
    ("136108-haumea", "Haumea", "136108;", ["Dwarf Planet", "TNO"]),
    ("Sedna", "Sedna", "90377;", ["TNO"]),
    ("50000-quaoar", "Quaoar", "50000;", ["TNO"]),
    ("90482-orcus", "Orcus", "90482;", ["TNO"]),
    ("225088-gonggong", "Gonggong", "225088;", ["TNO"]),
    ("28978-ixion", "Ixion", "28978;", ["TNO"]),
    ("20000-varuna", "Varuna", "20000;", ["TNO"]),
    ("486958-arrokoth", "Arrokoth", "486958;", ["TNO"]),
    ("38628-huya", "Huya", "38628;", ["TNO"]),
    ("174567-varda", "Varda", "174567;", ["TNO"]),
    ("120347-salacia", "Salacia", "120347;", ["TNO"]),
    ("2060-chiron", "Chiron", "2060;", ["Centaur"]),
    ("5145-pholus", "Pholus", "5145;", ["Centaur"]),
    ("10199-chariklo", "Chariklo", "10199;", ["Centaur"]),
]

COMETS = [
    ("1p-halley", "Halley", "DES=1P;CAP;", ["Comet"]),
    ("2p-encke", "Encke", "DES=2P;CAP;", ["Comet"]),
    ("9p-tempel1", "Tempel 1", "DES=9P;CAP;", ["Comet"]),
    ("19p-borrelly", "Borrelly", "DES=19P;CAP;", ["Comet"]),
    ("67p-cg", "Churyumov-Gerasimenko", "DES=67P;CAP;", ["Comet"]),
    ("81p-wild2", "Wild 2", "DES=81P;CAP;", ["Comet"]),
    ("103p-hartley2", "Hartley 2", "DES=103P;CAP;", ["Comet"]),
    ("c1995o1-halebopp", "Hale-Bopp", "C/1995 O1;", ["Comet"]),
]

INTERSTELLAR = [
    ("1i-oumuamua", "'Oumuamua", "1I;", ["Interstellar"]),
    ("2i-borisov", "Borisov", "2I;", ["Interstellar"]),
]

BARYCENTRIC = {"nix", "hydra", "kerberos", "styx"}  # orbit the Pluto-Charon barycentre
# Eris's moon. Its NAIF id is the Eris system id with a 120 prefix.
ERIS_MOON = [("dysnomia", "Dysnomia", "120136199", "eris")]

CENTER_OF = {"sol": "500@10", "eris": "500@20136199", "earth": "500@399", "mars": "500@499",
             "Jupiter": "500@599", "saturn": "500@699", "Uranus": "500@799",
             "Neptune": "500@899", "pluto": "500@999"}

def all_bodies():
    """(slug, name, command, primary, tags, center)"""
    out = [(*SUN, "500@10")]
    for slug, name, cmd, prim, tags in PLANETS:
        out.append((slug, name, cmd, prim, tags, CENTER_OF[prim]))
    for slug, name, cmd, prim in MOONS + ERIS_MOON:
        center = "500@9" if slug in BARYCENTRIC else CENTER_OF[prim]
        out.append((slug, name, cmd, prim, ["Moon"], center))
    for group in (ASTEROIDS, TNOS, COMETS, INTERSTELLAR):
        for slug, name, cmd, tags in group:
            out.append((slug, name, cmd, "sol", tags, "500@10"))
    return out

if __name__ == "__main__":
    b = all_bodies()
    print(f"{len(b)} bodies")
    from collections import Counter
    print(Counter(x[3] for x in b))
