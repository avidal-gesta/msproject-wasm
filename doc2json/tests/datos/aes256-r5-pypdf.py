"""Vector AES-256 revision 5 hecho por pypdf (implementacion independiente de doc2json).

pypdf necesita una biblioteca de AES que aca no esta instalada: se le da la primitiva AES de
Node (OpenSSL). La logica de la revision 5 (hash, /U, /UE, /O, /OE, /Perms) es la de pypdf.
Genera tests/datos/aes256-r5-pypdf.pdf (pypdf 6.11, Node 24); las sales son aleatorias, asi que cada
ejecucion da otro archivo igual de valido. Uso: python -I aes256-r5-pypdf.py salida.pdf
"""
import io, json, os, subprocess, sys
import pypdf._encryption as E
from pypdf import PdfWriter, PdfReader
from pypdf.generic import NameObject, DecodedStreamObject, DictionaryObject

NODE = r"""
const c = require('crypto'); const a = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const key = Buffer.from(a.key, 'hex'), iv = a.iv ? Buffer.from(a.iv, 'hex') : null;
const alg = `aes-${key.length * 8}-${a.mode}`;
const f = a.enc ? c.createCipheriv(alg, key, iv) : c.createDecipheriv(alg, key, iv);
f.setAutoPadding(!!a.pad);
process.stdout.write(Buffer.concat([f.update(Buffer.from(a.data, 'hex')), f.final()]).toString('hex'));
"""


def node(**a):
    a = {k: (v.hex() if isinstance(v, (bytes, bytearray)) else v) for k, v in a.items()}
    return bytes.fromhex(subprocess.run(['node', '-e', NODE], input=json.dumps(a), capture_output=True, text=True, check=True).stdout)


E.aes_cbc_encrypt = lambda key, iv, data: node(key=key, iv=iv, data=data, mode='cbc', enc=True)
E.aes_cbc_decrypt = lambda key, iv, data: node(key=key, iv=iv, data=data, mode='cbc', enc=False)
E.aes_ecb_encrypt = lambda key, data: node(key=key, data=data, mode='ecb', enc=True)
E.aes_ecb_decrypt = lambda key, data: node(key=key, data=data, mode='ecb', enc=False)


class CryptAES:
    def __init__(self, key):
        self.key = key

    def encrypt(self, data):
        iv = os.urandom(16)
        return iv + node(key=self.key, iv=iv, data=data, mode='cbc', enc=True, pad=True)

    def decrypt(self, data, *, strict=True):
        return node(key=self.key, iv=data[:16], data=data[16:], mode='cbc', enc=False, pad=True)


E.CryptAES = CryptAES

w = PdfWriter()
page = w.add_blank_page(200, 100)
font = DictionaryObject({NameObject('/Type'): NameObject('/Font'), NameObject('/Subtype'): NameObject('/Type1'), NameObject('/BaseFont'): NameObject('/Helvetica')})
page[NameObject('/Resources')] = DictionaryObject({NameObject('/Font'): DictionaryObject({NameObject('/F1'): w._add_object(font)})})
s = DecodedStreamObject()
s.set_data(b'BT /F1 12 Tf 10 50 Td (Hola pypdf R5) Tj ET')
page[NameObject('/Contents')] = w._add_object(s)
w.add_metadata({'/Title': 'Titulo pypdf'})
w.encrypt(user_password='', owner_password='propietario', algorithm='AES-256-R5')
buf = io.BytesIO()
w.write(buf)
data = buf.getvalue()
open(sys.argv[1], 'wb').write(data)
r = PdfReader(io.BytesIO(data))
enc = r.trailer['/Encrypt'].get_object()
print({k: (v if not isinstance(v, bytes) else len(v)) for k, v in enc.items()})
print('bytes', len(data), '| texto:', r.pages[0].extract_text(), '| titulo:', r.metadata.title)
